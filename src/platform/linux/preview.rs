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
    cards: HashMap<String, crate::platform::ControlCenterCard>,
}

pub(super) struct PreviewDesktop {
    listener: UnixListener,
    directory: PathBuf,
    children: HashMap<ProcessId, Application>,
    panel_requests: Vec<(ProcessId, Connection, Instant)>,
    panels: Vec<(ProcessId, ProcessId, Connection)>,
    pub creates: Vec<CreateWindowRequest>,
    pub closes: Vec<CloseWindowRequest>,
    pub exited: Vec<ProcessId>,
    pub dirty: bool,
    pub activations: Vec<ProcessId>,
    pub cancelled: Vec<CloseWindowRequest>,
    pub titles: Vec<(ProcessId, RemoteWindowId, String)>,
    pub posted_notifications: Vec<crate::platform::UserNotification>,
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
            panel_requests: Vec::new(),
            panels: Vec::new(),
            creates: Vec::new(),
            closes: Vec::new(),
            cancelled: Vec::new(),
            exited: Vec::new(),
            dirty: false,
            activations: Vec::new(),
            titles: Vec::new(),
            posted_notifications: Vec::new(),
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
        self.spawn(app, &[], None)
    }

    fn spawn(
        &mut self,
        app: &AppInfo,
        args: &[String],
        reply: Option<&std::path::Path>,
    ) -> Result<ProcessId, PlatformError> {
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
        let mut command = Command::new(executable);
        command.args(args);
        if let Some(reply) = reply {
            command.env("MOCHIOS_PREVIEW_FILE_PANEL_REPLY", reply);
        } else {
            command.env_remove("MOCHIOS_PREVIEW_FILE_PANEL_REPLY");
        }
        let child = command
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
                cards: HashMap::new(),
            },
        );
        Ok(pid)
    }

    pub fn control_center_cards(&self) -> Vec<crate::platform::ControlCenterCard> {
        self.children
            .values()
            .flat_map(|app| app.cards.values().cloned())
            .collect()
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
                        if self.panel_requests.len() < 64
                            && !self
                                .panel_requests
                                .iter()
                                .any(|(owner, _, _)| *owner == pid)
                            && !self.panels.iter().any(|(_, owner, _)| *owner == pid)
                        {
                            self.panel_requests.push((
                                pid,
                                Connection::new(stream)?,
                                Instant::now(),
                            ));
                        }
                        continue;
                    }
                    app.connection = Some(Rc::new(RefCell::new(Connection::new(stream)?)));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.poll_panels();
        let mut exited = Vec::new();
        for (&pid, app) in &mut self.children {
            if let Some(connection) = &app.connection {
                let packets = connection.borrow_mut().poll();
                match packets {
                    Ok(packets) => {
                        for packet in packets {
                            match packet {
                                Packet::UserNotification {
                                    id,
                                    bundle_id,
                                    title,
                                    body,
                                } if valid_notification(
                                    pid,
                                    &app.bundle_id,
                                    id,
                                    &bundle_id,
                                    &title,
                                    &body,
                                ) && self.posted_notifications.len() < 128 =>
                                {
                                    self.posted_notifications.push(
                                        crate::platform::UserNotification {
                                            id,
                                            bundle_id,
                                            title,
                                            body,
                                            read: false,
                                            created_at: SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_secs(),
                                        },
                                    );
                                }
                                Packet::ControlCenterCard {
                                    bundle_id,
                                    item_id,
                                    title,
                                    rows,
                                } if bundle_id == app.bundle_id
                                    && title.len() <= 96
                                    && !title.is_empty()
                                    && !title.chars().any(char::is_control)
                                    && (1..=4).contains(&rows.len())
                                    && rows.iter().all(|(label, value)| {
                                        !label.is_empty()
                                            && !value.is_empty()
                                            && !label.chars().any(char::is_control)
                                            && !value.chars().any(char::is_control)
                                    })
                                    && rows
                                        .iter()
                                        .map(|(a, b)| a.len() + b.len() + 2)
                                        .sum::<usize>()
                                        <= 2050
                                    && super::read_apps().iter().any(|catalog| {
                                        catalog.bundle_id == bundle_id
                                            && crate::ui::control_center::declares_item(
                                                catalog, &item_id,
                                            )
                                    }) =>
                                {
                                    app.cards.insert(
                                        item_id.clone(),
                                        crate::platform::ControlCenterCard {
                                            bundle_id,
                                            item_id,
                                            title,
                                            rows: rows
                                                .into_iter()
                                                .map(|(label, value)| {
                                                    crate::platform::ControlCenterCardRow {
                                                        label,
                                                        value,
                                                    }
                                                })
                                                .collect(),
                                        },
                                    );
                                    self.dirty = true;
                                }
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

    fn poll_panels(&mut self) {
        let mut waiting = Vec::new();
        for (owner, mut connection, started) in std::mem::take(&mut self.panel_requests) {
            if !self.children.contains_key(&owner) {
                continue;
            }
            match connection.poll() {
                Ok(packets)
                    if packets.is_empty() && started.elapsed() < Duration::from_secs(10) =>
                {
                    waiting.push((owner, connection, started));
                }
                Ok(mut packets) if packets.len() == 1 => {
                    let Packet::FilePanelRequest {
                        configuration,
                        reply_socket,
                    } = packets.remove(0)
                    else {
                        continue;
                    };
                    // A requester may expose only its private session reply socket.
                    let allowed =
                        fs::canonicalize(self.directory.join("panels").join(owner.0.to_string()))
                            .ok();
                    let Ok(socket) = fs::canonicalize(&reply_socket) else {
                        continue;
                    };
                    if configuration.len() > 65536
                        || !allowed.is_some_and(|root| socket.starts_with(root))
                    {
                        continue;
                    }
                    let Some(mut app) = super::read_apps()
                        .into_iter()
                        .find(|app| app.bundle_id == "org.mochios.files")
                    else {
                        let _ = connection.send(Packet::FilePanelOperation {
                            error: Some("Files.app is unavailable in this preview session.".into()),
                        });
                        continue;
                    };
                    app.bundle_id = format!("org.mochios.files.panel.{}", owner.0);
                    let payload: String = configuration
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    let argument = format!("--system-file-panel=0:{}:{payload}", "00".repeat(16));
                    match self.spawn(&app, &[argument], Some(&socket)) {
                        Ok(child) => self.panels.push((child, owner, connection)),
                        Err(_) => {
                            let _ = connection.send(Packet::FilePanelOperation {
                                error: Some("Files.app could not be started.".into()),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        self.panel_requests = waiting;
        let mut active = Vec::new();
        for (child, owner, mut connection) in std::mem::take(&mut self.panels) {
            if !self.children.contains_key(&child) {
                continue;
            }
            if !self.children.contains_key(&owner) || connection.poll().is_err() {
                self.terminate(child);
            } else {
                active.push((child, owner, connection));
            }
        }
        self.panels = active;
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

fn valid_notification(
    pid: ProcessId,
    owner: &str,
    id: u64,
    bundle: &str,
    title: &str,
    body: &str,
) -> bool {
    bundle == owner
        && id >> 32 == pid.0 as u64
        && id as u32 != 0
        && !title.is_empty()
        && title.len() <= 128
        && !title.chars().any(char::is_control)
        && !body.is_empty()
        && body.len() <= 1024
        && !body
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn preview_notifications_require_the_authenticated_owner_and_valid_text() {
        let pid = ProcessId(42);
        let id = (42u64 << 32) | 1;
        assert!(valid_notification(
            pid,
            "org.example.app",
            id,
            "org.example.app",
            "完了",
            "保存しました\n次の操作"
        ));
        assert!(!valid_notification(
            pid,
            "org.example.app",
            id,
            "org.other.app",
            "Done",
            "Body"
        ));
        assert!(!valid_notification(
            pid,
            "org.example.app",
            (43u64 << 32) | 1,
            "org.example.app",
            "Done",
            "Body"
        ));
        assert!(!valid_notification(
            pid,
            "org.example.app",
            id,
            "org.example.app",
            "",
            "Body"
        ));
        assert!(!valid_notification(
            pid,
            "org.example.app",
            id,
            "org.example.app",
            "Done",
            "Bad\0body"
        ));
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
        let _ = fs::remove_dir_all(&self.directory);
    }
}
