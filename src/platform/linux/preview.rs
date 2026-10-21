//! Real host applications rendered into Binder's existing window layer.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::transport::peer_process_id;
use crate::platform::{
    AppInfo, CloseWindowRequest, CreateWindowRequest, PlatformError, ProcessId, RemoteWindowId,
};
use viewkit::draw_command::DisplayList;
use viewkit::platform::linux::preview::{Connection, Packet, SOCKET_ENV};
use viewkit::platform::{CursorIcon, PlatformEvent};

pub(crate) struct Surface {
    pub list: DisplayList,
    pub cursor: CursorIcon,
    pub connection: Rc<RefCell<Connection>>,
    pub local_id: u64,
}

thread_local! {
    static SURFACES: RefCell<HashMap<String, Rc<RefCell<Surface>>>> = RefCell::new(HashMap::new());
}

pub(crate) fn surface(renderer: &str) -> Option<Rc<RefCell<Surface>>> {
    SURFACES.with(|surfaces| surfaces.borrow().get(renderer).cloned())
}

fn renderer(pid: ProcessId, id: u64) -> String {
    format!("host-preview:{}:{id}", pid.0)
}

struct Application {
    child: Child,
    bundle_id: String,
    started: Instant,
    connection: Option<Rc<RefCell<Connection>>>,
    pending: VecDeque<u64>,
    windows: HashMap<RemoteWindowId, u64>,
    local_windows: HashSet<u64>,
}

pub(super) struct PreviewDesktop {
    listener: UnixListener,
    directory: PathBuf,
    children: HashMap<ProcessId, Application>,
    pub creates: Vec<CreateWindowRequest>,
    pub closes: Vec<CloseWindowRequest>,
    pub exited: Vec<ProcessId>,
    pub dirty: bool,
    pub activations: Vec<ProcessId>,
    pub cancelled: Vec<CloseWindowRequest>,
    pub titles: Vec<(ProcessId, RemoteWindowId, String)>,
}

impl PreviewDesktop {
    pub fn new() -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("mochios-ui-{}-{nonce}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let listener = UnixListener::bind(directory.join("viewkit.sock"))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            directory,
            children: HashMap::new(),
            creates: Vec::new(),
            closes: Vec::new(),
            cancelled: Vec::new(),
            exited: Vec::new(),
            dirty: false,
            activations: Vec::new(),
            titles: Vec::new(),
        })
    }

    pub fn process_id(&self, bundle_id: &str) -> Option<ProcessId> {
        self.children
            .iter()
            .find_map(|(pid, app)| (app.bundle_id == bundle_id).then_some(*pid))
    }

    pub fn bundle_ids(&self) -> Vec<String> {
        self.children
            .values()
            .map(|app| app.bundle_id.clone())
            .collect()
    }

    pub fn contains(&self, pid: ProcessId) -> bool {
        self.children.contains_key(&pid)
    }

    pub fn launch(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        if let Some(pid) = self.process_id(&app.bundle_id) {
            self.activations.push(pid);
            return Ok(pid);
        }
        // Executables come only from the session's generated application catalogue.
        let executable = app.entry_path();
        let root =
            std::env::var_os("MOCHIOS_PREVIEW_APPS").ok_or(PlatformError::UnsupportedOperation)?;
        let root = fs::canonicalize(root).map_err(|_| PlatformError::ProcessLaunchFailed)?;
        let executable =
            fs::canonicalize(executable).map_err(|_| PlatformError::ProcessLaunchFailed)?;
        if !executable.starts_with(&root) {
            return Err(PlatformError::PermissionDenied);
        }
        let child = Command::new(executable)
            .env(SOCKET_ENV, self.directory.join("viewkit.sock"))
            .env_remove("BINDER_SOCKET")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| PlatformError::ProcessLaunchFailed)?;
        let pid = ProcessId(child.id());
        self.children.insert(
            pid,
            Application {
                child,
                bundle_id: app.bundle_id.clone(),
                started: Instant::now(),
                connection: None,
                pending: VecDeque::new(),
                windows: HashMap::new(),
                local_windows: HashSet::new(),
            },
        );
        Ok(pid)
    }

    pub fn register(
        &mut self,
        pid: ProcessId,
        remote: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        let app = self
            .children
            .get_mut(&pid)
            .ok_or(PlatformError::ServiceUnavailable)?;
        let local = app
            .pending
            .pop_front()
            .ok_or(PlatformError::InvalidResponse)?;
        app.windows.insert(remote, local);
        Ok(())
    }

    pub fn event(
        &mut self,
        pid: ProcessId,
        remote: RemoteWindowId,
        event: PlatformEvent,
    ) -> Result<(), PlatformError> {
        let app = self
            .children
            .get_mut(&pid)
            .ok_or(PlatformError::ServiceUnavailable)?;
        let id = *app
            .windows
            .get(&remote)
            .ok_or(PlatformError::PermissionDenied)?;
        app.connection
            .as_ref()
            .ok_or(PlatformError::ServiceUnavailable)?
            .borrow_mut()
            .send(Packet::Event { id, event })
            .map_err(|_| PlatformError::TransportFailure)
    }

    pub fn poll(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let Ok(pid) = peer_process_id(&stream) else {
                        continue;
                    };
                    let Some(app) = self.children.get_mut(&pid) else {
                        continue;
                    };
                    if app.connection.is_some() {
                        continue;
                    }
                    app.connection = Some(Rc::new(RefCell::new(Connection::new(stream)?)));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut exited = Vec::new();
        for (&pid, app) in &mut self.children {
            if let Some(connection) = &app.connection {
                let packets = connection.borrow_mut().poll();
                match packets {
                    Ok(packets) => {
                        for packet in packets {
                            match packet {
                                Packet::Open { id, config }
                                    if id != 0
                                        && app.local_windows.len() < 64
                                        && !config.title.is_empty()
                                        && config.title.len() <= 1024
                                        && config.size.width.is_finite()
                                        && config.size.height.is_finite()
                                        && (1.0..=4096.0).contains(&config.size.width)
                                        && (1.0..=4096.0).contains(&config.size.height)
                                        && app.local_windows.insert(id) =>
                                {
                                    let key = renderer(pid, id);
                                    SURFACES.with(|surfaces| {
                                        surfaces.borrow_mut().insert(
                                            key.clone(),
                                            Rc::new(RefCell::new(Surface {
                                                list: DisplayList::new(),
                                                cursor: CursorIcon::Default,
                                                connection: Rc::clone(connection),
                                                local_id: id,
                                            })),
                                        );
                                    });
                                    app.pending.push_back(id);
                                    self.creates.push(CreateWindowRequest {
                                        process_id: pid,
                                        renderer: key,
                                        title: config.title,
                                        width: config.size.width.round() as u32,
                                        height: config.size.height.round() as u32
                                            + crate::ui::window_decoration::TITLE_BAR_HEIGHT as u32,
                                        resizable: config.resizable,
                                    });
                                }
                                Packet::Frame { id, list, cursor }
                                    if app.local_windows.contains(&id) =>
                                {
                                    if let Some(surface) = surface(&renderer(pid, id)) {
                                        // Clip depth may never escape the application's content surface.
                                        if !valid_list(&list) {
                                            exited.push(pid);
                                            break;
                                        }
                                        let mut surface = surface.borrow_mut();
                                        surface.list = list;
                                        surface.cursor = cursor;
                                        self.dirty = true;
                                    }
                                }
                                Packet::Close { id } if app.local_windows.remove(&id) => {
                                    SURFACES.with(|surfaces| {
                                        surfaces.borrow_mut().remove(&renderer(pid, id));
                                    });
                                    if let Some(remote) =
                                        app.windows.iter().find_map(|(remote, local)| {
                                            (*local == id).then_some(*remote)
                                        })
                                    {
                                        app.windows.remove(&remote);
                                        self.closes.push(CloseWindowRequest {
                                            process_id: pid,
                                            window: remote,
                                        });
                                    }
                                    app.pending.retain(|pending| *pending != id);
                                    self.creates
                                        .retain(|request| request.renderer != renderer(pid, id));
                                }
                                Packet::CloseCancelled { id } => {
                                    if let Some(remote) =
                                        app.windows.iter().find_map(|(remote, local)| {
                                            (*local == id).then_some(*remote)
                                        })
                                    {
                                        self.cancelled.push(CloseWindowRequest {
                                            process_id: pid,
                                            window: remote,
                                        });
                                    }
                                }
                                Packet::Title { id, title } if title.len() <= 1024 => {
                                    if let Some(remote) =
                                        app.windows.iter().find_map(|(remote, local)| {
                                            (*local == id).then_some(*remote)
                                        })
                                    {
                                        self.titles.push((pid, remote, title));
                                        self.dirty = true;
                                    }
                                }
                                _ => {
                                    exited.push(pid);
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => exited.push(pid),
                }
            }
            if app.child.try_wait()?.is_some()
                || (app.connection.is_none() && app.started.elapsed() > Duration::from_secs(10))
            {
                exited.push(pid);
            }
        }
        exited.sort_by_key(|pid| pid.0);
        exited.dedup();
        for pid in exited {
            self.terminate(pid);
        }
        Ok(())
    }

    fn terminate(&mut self, pid: ProcessId) {
        if let Some(mut app) = self.children.remove(&pid) {
            let _ = app.child.kill();
            let _ = app.child.wait();
            SURFACES.with(|surfaces| {
                let mut surfaces = surfaces.borrow_mut();
                for id in app.local_windows {
                    surfaces.remove(&renderer(pid, id));
                }
            });
            self.exited.push(pid);
        }
    }
}

fn valid_list(list: &DisplayList) -> bool {
    use viewkit::draw_command::DrawCommand;
    let mut depth = 0;
    for command in list.commands() {
        match command {
            DrawCommand::PushClip { .. } | DrawCommand::PushRoundedClip { .. } => depth += 1,
            DrawCommand::PopClip => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
        if depth > 256 {
            return false;
        }
    }
    depth == 0
}

impl Drop for PreviewDesktop {
    fn drop(&mut self) {
        let pids: Vec<_> = self.children.keys().copied().collect();
        for pid in pids {
            self.terminate(pid);
        }
        let _ = fs::remove_file(self.directory.join("viewkit.sock"));
        let _ = fs::remove_dir(&self.directory);
    }
}
