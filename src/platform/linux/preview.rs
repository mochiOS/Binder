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
    pub has_frame: bool,
    pub cursor: CursorIcon,
    connection: Rc<RefCell<Connection>>,
    local_id: u64,
    process_id: ProcessId,
    input: Rc<RefCell<InputAuthorization>>,
}

/// Shared by native host surfaces and platform-generated focus/resize events.
/// Every actual press must pass here before a child may request a gesture.
#[derive(Default)]
struct InputAuthorization {
    presses: HashMap<(ProcessId, u64), (u64, viewkit::geometry::Point, bool)>,
    pointers: HashMap<(ProcessId, u64), viewkit::geometry::Point>,
    next_serial: u64,
}
impl InputAuthorization {
    fn packet(&mut self, pid: ProcessId, id: u64, event: PlatformEvent) -> Packet {
        let mut serial = 0;
        let key = (pid, id);
        match &event {
            PlatformEvent::PointerMoved { x, y } => {
                self.pointers
                    .insert(key, viewkit::geometry::Point::new(*x, *y));
            }
            PlatformEvent::PointerButton {
                button: viewkit::platform::PointerButton::Primary,
                state: viewkit::platform::ButtonState::Pressed,
            } => {
                self.next_serial = self.next_serial.wrapping_add(1).max(1);
                serial = self.next_serial;
                self.presses.insert(
                    key,
                    (
                        serial,
                        self.pointers.get(&key).copied().unwrap_or_default(),
                        false,
                    ),
                );
            }
            PlatformEvent::PointerButton {
                button: viewkit::platform::PointerButton::Primary,
                state: viewkit::platform::ButtonState::Released,
            }
            | PlatformEvent::Focused(false) => {
                self.presses.remove(&key);
            }
            _ => {}
        }
        if serial != 0 {
            Packet::Input { id, serial, event }
        } else {
            Packet::Event { id, event }
        }
    }
}
impl Surface {
    pub(crate) fn send_event(&self, event: PlatformEvent) -> io::Result<()> {
        let packet = self
            .input
            .borrow_mut()
            .packet(self.process_id, self.local_id, event);
        self.connection.borrow_mut().send(packet)
    }
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
    pub fullscreen_work_area: Option<viewkit::geometry::Rect>,
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
    decorations: HashMap<(ProcessId, u64), viewkit::decoration::DecorationConfig>,
    pending_decorations: HashMap<(ProcessId, u64), viewkit::decoration::DecorationConfig>,
    input: Rc<RefCell<InputAuthorization>>,
    layouts: HashMap<(ProcessId, u64), viewkit::decoration::DecorationLayout>,
    gestures: Vec<(ProcessId, u64, u64, viewkit::geometry::Point, u32)>,
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
            fullscreen_work_area: None,
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
            decorations: HashMap::new(),
            pending_decorations: HashMap::new(),
            input: Rc::new(RefCell::new(InputAuthorization::default())),
            layouts: HashMap::new(),
            gestures: Vec::new(),
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
        command.env(
            viewkit::environment::INPUT_MODE_ENV,
            if self.fullscreen_work_area.is_some() {
                "touch"
            } else {
                "pointer"
            },
        );
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
        let packet = self.input.borrow_mut().packet(pid, id, event);
        app.connection
            .as_ref()
            .ok_or(PlatformError::ServiceUnavailable)?
            .borrow_mut()
            .send(packet)
            .map_err(|_| PlatformError::TransportFailure)
    }

    pub fn layout(
        &mut self,
        pid: ProcessId,
        remote: RemoteWindowId,
        layout: viewkit::decoration::DecorationLayout,
    ) -> Result<(), PlatformError> {
        let Some(app) = self.children.get(&pid) else {
            return Ok(());
        };
        let Some(&id) = app.windows.get(&remote) else {
            return Ok(());
        };
        if self.layouts.get(&(pid, id)) == Some(&layout) {
            return Ok(());
        }
        app.connection
            .as_ref()
            .ok_or(PlatformError::ServiceUnavailable)?
            .borrow_mut()
            .send(Packet::DecorationLayout { id, layout })
            .map_err(|_| PlatformError::TransportFailure)?;
        self.layouts.insert((pid, id), layout);
        Ok(())
    }
    pub fn decoration(
        &self,
        pid: ProcessId,
        remote: RemoteWindowId,
    ) -> Option<viewkit::decoration::DecorationConfig> {
        let local = *self.children.get(&pid)?.windows.get(&remote)?;
        self.decorations.get(&(pid, local)).copied()
    }
    pub fn take_gestures(
        &mut self,
    ) -> Vec<(ProcessId, RemoteWindowId, viewkit::geometry::Point, u32)> {
        let mut ready = Vec::new();
        for (pid, id, serial, point, edge) in self.gestures.drain(..) {
            if self
                .input
                .borrow()
                .presses
                .get(&(pid, id))
                .is_some_and(|(active, _, used)| *active == serial && *used)
            {
                if let Some(remote) = self.children.get(&pid).and_then(|app| {
                    app.windows
                        .iter()
                        .find_map(|(r, l)| (*l == id).then_some(*r))
                }) {
                    ready.push((pid, remote, point, edge));
                }
            }
        }
        ready
    }
    pub fn take_decorations(
        &mut self,
    ) -> Vec<(
        ProcessId,
        RemoteWindowId,
        viewkit::decoration::DecorationConfig,
    )> {
        let mut ready = Vec::new();
        let mut pending = HashMap::new();
        for ((pid, id), config) in self.pending_decorations.drain() {
            let remote = self.children.get(&pid).and_then(|app| {
                app.windows
                    .iter()
                    .find_map(|(r, l)| (*l == id).then_some(*r))
            });
            if let Some(remote) = remote {
                ready.push((pid, remote, config));
            } else {
                pending.insert((pid, id), config);
            }
        }
        self.pending_decorations = pending;
        ready
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
                                Packet::Decoration { id, config }
                                    if app.local_windows.contains(&id)
                                        && config.height.is_finite()
                                        && (1.0..=4096.0).contains(&config.height) =>
                                {
                                    self.decorations.insert((pid, id), config);
                                    self.pending_decorations.insert((pid, id), config);
                                    self.dirty = true;
                                }
                                Packet::WindowInteraction { id, serial, edge }
                                    if app.local_windows.contains(&id)
                                        && matches!(edge, 0 | 1 | 2 | 4 | 8 | 5 | 6 | 9 | 10) =>
                                {
                                    let mut input = self.input.borrow_mut();
                                    if input.presses.get(&(pid, id)).is_some_and(|(s, _, used)| {
                                        *s == serial && serial != 0 && !*used
                                    }) {
                                        let press = input.presses.get_mut(&(pid, id)).unwrap();
                                        press.2 = true;
                                        let point = press.1;
                                        if let Some(remote) = app
                                            .windows
                                            .iter()
                                            .find_map(|(r, l)| (*l == id).then_some(*r))
                                        {
                                            self.gestures.push((pid, id, serial, point, edge));
                                            self.dirty = true;
                                        }
                                    }
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
                                    let size = self.fullscreen_work_area.map_or(config.size, |area| area.size);
                                    if self.fullscreen_work_area.is_some() {
                                        // Negotiate the shell's initial viewport before the
                                        // normal window-model registration round trip.
                                        let resized = PlatformEvent::Resized {
                                            viewport: viewkit::platform::linux::preview::viewport(
                                                size.width.round().max(1.0) as u32,
                                                size.height.round().max(1.0) as u32,
                                            ),
                                        };
                                        if connection.borrow_mut().send(Packet::Event { id, event: resized }).is_err() {
                                            exited.push(pid);
                                            break;
                                        }
                                    }
                                    SURFACES.with(|surfaces| {
                                        surfaces.borrow_mut().insert(
                                            key.clone(),
                                            Rc::new(RefCell::new(Surface {
                                                list: DisplayList::new(),
                                                has_frame: false,
                                                cursor: CursorIcon::Default,
                                                connection: Rc::clone(connection),
                                                local_id: id,
                                                process_id: pid,
                                                input: Rc::clone(&self.input),
                                            })),
                                        );
                                    });
                                    app.pending.push_back(id);
                                    self.creates.push(CreateWindowRequest {
                                        process_id: pid,
                                        renderer: key,
                                        title: config.title,
                                        width: size.width.round() as u32,
                                        height: size.height.round() as u32
                                            + if self.fullscreen_work_area.is_some(){0}else{crate::ui::window_decoration::TITLE_BAR_HEIGHT as u32},
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
                                        surface.has_frame = true;
                                        surface.cursor = cursor;
                                        self.dirty = true;
                                    }
                                }
                                Packet::Close { id } if app.local_windows.remove(&id) => {
                                    self.decorations.remove(&(pid, id));
                                    self.pending_decorations.remove(&(pid, id));
                                    self.input.borrow_mut().presses.remove(&(pid, id));
                                    self.input.borrow_mut().pointers.remove(&(pid, id));
                                    self.layouts.remove(&(pid, id));
                                    self.gestures
                                        .retain(|(p, i, _, _, _)| (*p, *i) != (pid, id));

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

    fn open_associated_document(
        &mut self,
        path: &std::path::Path,
        content_type: &str,
        bundle_id: &str,
        roles: u16,
    ) -> Result<ProcessId, String> {
        if roles == 0 || roles & !3 != 0 || path.as_os_str().len() > 4096 || !path.is_file() {
            return Err("The selected document is unavailable or unsupported.".into());
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let app = super::read_apps()
            .into_iter()
            .find(|app| {
                (bundle_id.is_empty() || app.bundle_id == bundle_id)
                    && fs::read_to_string(app.root.join("manifest.toml")).is_ok_and(|manifest| {
                        manifest_handles_document(&manifest, extension, content_type, roles)
                    })
            })
            .ok_or("No installed preview application handles this document.")?;
        let path = path
            .to_str()
            .ok_or("The document path is not valid UTF-8.")?;
        self.spawn(&app, &[path.into()], None)
            .map_err(|_| "The document application could not be started.".into())
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
                    let packet = packets.remove(0);
                    if let Packet::OpenDocument {
                        path,
                        content_type,
                        bundle_id,
                        roles,
                    } = packet
                    {
                        let result =
                            self.open_associated_document(&path, &content_type, &bundle_id, roles);
                        let (process_id, error) = match result {
                            Ok(pid) => (Some(pid.0), None),
                            Err(error) => (None, Some(error)),
                        };
                        let _ = connection.send(Packet::DocumentOpened { process_id, error });
                        continue;
                    }
                    let Packet::FilePanelRequest {
                        configuration,
                        reply_socket,
                    } = packet
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
        self.decorations.retain(|(p, _), _| *p != pid);
        self.pending_decorations.retain(|(p, _), _| *p != pid);
        self.input
            .borrow_mut()
            .presses
            .retain(|(p, _), _| *p != pid);
        self.input
            .borrow_mut()
            .pointers
            .retain(|(p, _), _| *p != pid);
        self.layouts.retain(|(p, _), _| *p != pid);
        self.gestures.retain(|(p, _, _, _, _)| *p != pid);
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

fn manifest_handles_document(
    manifest: &str,
    extension: &str,
    content_type: &str,
    roles: u16,
) -> bool {
    let declared = super::parse_string_array_field(manifest, "document_roles")
        .iter()
        .fold(0, |bits, role| {
            bits | match role.as_str() {
                "view" => 1,
                "edit" => 2,
                _ => 0,
            }
        });
    roles != 0
        && roles & !3 == 0
        && declared & roles == roles
        && (super::parse_string_array_field(manifest, "document_extensions")
            .iter()
            .any(|value| {
                value
                    .trim_start_matches('.')
                    .eq_ignore_ascii_case(extension)
            })
            || super::parse_string_array_field(manifest, "document_content_types")
                .iter()
                .any(|value| value.eq_ignore_ascii_case(content_type)))
}

fn valid_list(list: &DisplayList) -> bool {
    use viewkit::draw_command::DrawCommand;
    let mut depth = 0;
    for command in list.commands() {
        match command {
            DrawCommand::PushClip { .. }
            | DrawCommand::PushRoundedClip { .. }
            | DrawCommand::PushRoundedCornersClip { .. } => depth += 1,
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

#[cfg(test)]
mod document_open_tests {
    use super::*;

    #[test]
    fn preview_dispatch_uses_manifest_document_associations() {
        let manifest = include_str!("../../../../fonts/manifest.toml");
        for (extension, mime) in [
            ("TTF", "font/ttf"),
            ("otf", "font/otf"),
            ("ttc", "font/collection"),
            ("otc", "font/collection"),
        ] {
            assert!(manifest_handles_document(manifest, extension, mime, 1));
        }
        assert!(!manifest_handles_document(manifest, "png", "image/png", 1));
        assert!(!manifest_handles_document(manifest, "ttf", "font/ttf", 2));
        assert!(!manifest_handles_document(manifest, "ttf", "font/ttf", 0));
    }

    #[test]
    fn asymmetric_list_row_clips_are_balanced_in_preview_scenes() {
        let mut list = DisplayList::new();
        list.push(viewkit::draw_command::DrawCommand::PushRoundedCornersClip {
            rect: viewkit::prelude::Rect::new(0.0, 0.0, 120.0, 48.0),
            radii: [0.0, 0.0, 6.0, 6.0],
        });
        list.push(viewkit::draw_command::DrawCommand::PopClip);
        assert!(valid_list(&list));
    }

    #[test]
    #[ignore = "subprocess fixture for the document-open transport test"]
    fn document_request_child() {
        let socket = std::env::var_os(SOCKET_ENV).unwrap();
        let _application_connection =
            Connection::new(std::os::unix::net::UnixStream::connect(socket).unwrap()).unwrap();
        let path = PathBuf::from(std::env::var_os("MOCHIOS_VIEWER_SMOKE_IMAGE").unwrap());
        let pid = viewkit::platform::linux::preview::open_document(&path, "image/svg+xml", "", 1)
            .unwrap();
        fs::write(path.with_extension("result"), pid.to_string()).unwrap();
    }

    #[test]
    #[ignore = "requires mmake ui-check binaries and a generated MOCHIOS_PREVIEW_APPS catalogue"]
    fn authenticated_file_open_renders_the_real_viewer_document_without_a_host_display() {
        let mut desktop = PreviewDesktop::new().unwrap();
        let path = desktop.directory.join("viewer-open.svg");
        fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::linux::preview::document_open_tests::document_request_child",
                "--ignored",
            ])
            .env(SOCKET_ENV, desktop.directory.join("viewkit.sock"))
            .env("MOCHIOS_VIEWER_SMOKE_IMAGE", &path)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let owner = ProcessId(child.id());
        desktop.children.insert(
            owner,
            Application {
                child,
                bundle_id: "org.mochios.files".into(),
                started: Instant::now(),
                connection: None,
                pending: VecDeque::new(),
                windows: HashMap::new(),
                local_windows: HashSet::new(),
                cards: HashMap::new(),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut rendered = false;
        while Instant::now() < deadline {
            desktop.poll().unwrap();
            rendered = desktop.creates.iter().filter(|request| request.process_id != owner && request.title.contains("viewer-open.svg"))
                .any(|request| surface(&request.renderer).is_some_and(|surface| surface.borrow().list.commands().iter().any(|command|
                    matches!(command, viewkit::draw_command::DrawCommand::DrawSvg { command } if command.svg.size() == (20.0, 10.0)))));
            if rendered && path.with_extension("result").is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            rendered,
            "the real Viewer must decode and draw the selected image"
        );
        let pid = fs::read_to_string(path.with_extension("result"))
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(desktop.children.contains_key(&ProcessId(pid)));
    }
}

#[cfg(test)]
mod decoration_admission_tests {
    use super::*;
    fn window() -> (PreviewDesktop, ProcessId, Connection) {
        let mut desktop = PreviewDesktop::new().unwrap();
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = ProcessId(child.id());
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        desktop.children.insert(
            pid,
            Application {
                child,
                bundle_id: "test.decoration".into(),
                started: Instant::now(),
                connection: Some(Rc::new(RefCell::new(Connection::new(server).unwrap()))),
                pending: VecDeque::new(),
                windows: HashMap::from([(RemoteWindowId(7), 1)]),
                local_windows: HashSet::from([1]),
                cards: HashMap::new(),
            },
        );
        (desktop, pid, Connection::new(client).unwrap())
    }
    #[test]
    fn host_surface_input_issues_authorization_used_by_real_gesture_admission() {
        use viewkit::{
            event::{EventContext, ViewEvent},
            geometry::{Point, Rect},
            platform::PointerButton,
            theme::Theme,
            typography::TextMeasurer,
            view::View,
        };
        let (mut desktop, pid, mut client) = window();
        let connection = desktop.children[&pid].connection.as_ref().unwrap().clone();
        let key = renderer(pid, 1);
        SURFACES.with(|surfaces| {
            surfaces.borrow_mut().insert(
                key.clone(),
                Rc::new(RefCell::new(Surface {
                    list: DisplayList::new(),
                    has_frame: false,
                    cursor: CursorIcon::Default,
                    connection,
                    local_id: 1,
                    process_id: pid,
                    input: desktop.input.clone(),
                })),
            )
        });
        let host = crate::ui::host_surface::HostSurface::for_renderer(&key).unwrap();
        let bounds = Rect::new(100.0, 200.0, 800.0, 600.0);
        let point = Point::new(160.0, 220.0);
        let theme = Theme::LIGHT;
        let mut text = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text);
        host.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position: point,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        let serial = client
            .poll()
            .unwrap()
            .into_iter()
            .find_map(|packet| match packet {
                Packet::Input {
                    id: 1,
                    serial,
                    event:
                        PlatformEvent::PointerButton {
                            button: PointerButton::Primary,
                            state: viewkit::platform::ButtonState::Pressed,
                        },
                } => Some(serial),
                _ => None,
            })
            .expect("the real HostSurface path must issue an authenticated Input packet");
        assert_ne!(serial, 0);
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        let gestures = desktop.take_gestures();
        assert_eq!(
            gestures,
            vec![(pid, RemoteWindowId(7), Point::new(60.0, 20.0), 0)]
        );
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        assert!(
            desktop.take_gestures().is_empty(),
            "a consumed press cannot be replayed"
        );
        host.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position: point,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        client.poll().unwrap();
        host.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position: point,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        let serial = client
            .poll()
            .unwrap()
            .into_iter()
            .find_map(|packet| match packet {
                Packet::Input { serial, .. } => Some(serial),
                _ => None,
            })
            .unwrap();
        host.handle_event(
            bounds,
            &ViewEvent::FocusChanged { focused: false },
            &mut context,
        );
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        assert!(
            desktop.take_gestures().is_empty(),
            "host focus loss revokes the same authorization"
        );
    }

    #[test]
    fn release_invalidates_even_an_already_queued_gesture_and_replays_are_denied() {
        let (mut desktop, pid, mut client) = window();
        let point = viewkit::geometry::Point::new(10.0, 20.0);
        desktop
            .input
            .borrow_mut()
            .presses
            .insert((pid, 1), (73, point, false));
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial: 73,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        assert_eq!(desktop.take_gestures().len(), 1);
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial: 73,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        assert!(desktop.take_gestures().is_empty());
        desktop
            .input
            .borrow_mut()
            .presses
            .insert((pid, 1), (74, point, false));
        client
            .send(Packet::WindowInteraction {
                id: 1,
                serial: 74,
                edge: 0,
            })
            .unwrap();
        desktop.poll().unwrap();
        desktop
            .event(
                pid,
                RemoteWindowId(7),
                PlatformEvent::PointerButton {
                    button: viewkit::platform::PointerButton::Primary,
                    state: viewkit::platform::ButtonState::Released,
                },
            )
            .unwrap();
        assert!(desktop.take_gestures().is_empty());
    }
    #[test]
    fn pending_decoration_configuration_is_coalesced_per_window() {
        let (mut desktop, _pid, mut client) = window();
        for height in 40..1000 {
            client
                .send(Packet::Decoration {
                    id: 1,
                    config: viewkit::decoration::DecorationConfig {
                        mode: viewkit::decoration::DecorationMode::Integrated,
                        height: height as f32,
                        background: viewkit::theme::Color::BLACK,
                        separator: true,
                        resizable: true,
                    },
                })
                .unwrap();
        }
        // Nonblocking writes may span several poll budgets. Drain the real
        // transport before asserting that its latest configuration won.
        for _ in 0..100 {
            client.poll().unwrap();
            desktop.poll().unwrap();
            if desktop
                .decorations
                .values()
                .any(|config| config.height == 999.0)
            {
                break;
            }
        }
        let configurations = desktop.take_decorations();
        assert_eq!(configurations.len(), 1);
        assert_eq!(configurations[0].2.height, 999.0);
    }
}
