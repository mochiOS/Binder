mod transport;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ipc::{ClientRequest, RemoteWindowId, ServerEvent};

use super::{
    AppInfo, ClockState, CloseWindowRequest, CreateWindowRequest, DesktopPlatform, PlatformError,
    ProcessId, SystemAction, SystemBarState, WindowFocusChangedNotification,
    WindowResizedNotification,
};

use crate::window::WindowContent;
use transport::{BINDER_SOCKET_ENV, LinuxIpcServer, TransportEvent};

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

const ORPHAN_TIMEOUT: Duration = Duration::from_secs(1);

const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InternalAppLaunch {
    entry: &'static str,
    role_argument: &'static str,
    default_bundle_id: &'static str,
    content: WindowContent,
}

const INTERNAL_ABOUT_APP: InternalAppLaunch = InternalAppLaunch {
    entry: "internal:about",
    role_argument: "--role=about",
    default_bundle_id: "com.mochi.binder.about",
    content: WindowContent::About,
};

const INTERNAL_TEST_APP: InternalAppLaunch = InternalAppLaunch {
    entry: "internal:test",
    role_argument: "--role=test",
    default_bundle_id: "com.mochi.binder.test",
    content: WindowContent::Test,
};

struct ManagedChild {
    content: WindowContent,
    bundle_id: String,
    child: Child,

    launched_at: Instant,
    registered_at: Option<Instant>,
    disconnected_at: Option<Instant>,

    windows: HashSet<RemoteWindowId>,

    close_deadlines: HashMap<RemoteWindowId, Instant>,
}

pub struct LinuxPlatform {
    system_bar: SystemBarState,

    transport: LinuxIpcServer,

    children: HashMap<ProcessId, ManagedChild>,

    create_window_requests: Vec<CreateWindowRequest>,

    close_window_requests: Vec<CloseWindowRequest>,

    exited_processes: Vec<ProcessId>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self::try_new().unwrap_or_else(|error| {
            panic!("failed to initialize Binder platform: {error:?}",);
        })
    }

    fn try_new() -> Result<Self, PlatformError> {
        Ok(Self {
            system_bar: read_system_bar_state().unwrap_or_default(),

            transport: LinuxIpcServer::new()?,

            children: HashMap::new(),

            create_window_requests: Vec::new(),

            close_window_requests: Vec::new(),

            exited_processes: Vec::new(),
        })
    }

    fn reap_exited_children(&mut self) -> Result<(), PlatformError> {
        let mut exited = Vec::new();

        for (process_id, managed_child) in &mut self.children {
            match managed_child.child.try_wait() {
                Ok(Some(_)) => {
                    exited.push(*process_id);
                }

                Ok(None) => {}

                Err(_) => {
                    return Err(PlatformError::TransportFailure);
                }
            }
        }

        for process_id in exited {
            self.children.remove(&process_id);

            self.transport.remove_client(process_id);

            self.exited_processes.push(process_id);
        }

        Ok(())
    }

    fn terminate_child(&mut self, process_id: ProcessId) -> Result<(), PlatformError> {
        let Some(mut managed_child) = self.children.remove(&process_id) else {
            return Ok(());
        };

        self.transport.remove_client(process_id);

        match managed_child.child.try_wait() {
            Ok(Some(_)) => {}

            Ok(None) => {
                managed_child
                    .child
                    .kill()
                    .map_err(|_| PlatformError::ProcessTerminationFailed)?;

                managed_child
                    .child
                    .wait()
                    .map_err(|_| PlatformError::ProcessTerminationFailed)?;
            }

            Err(_) => {
                return Err(PlatformError::ProcessTerminationFailed);
            }
        }

        self.exited_processes.push(process_id);

        Ok(())
    }

    fn handle_transport_event(&mut self, event: TransportEvent) {
        match event {
            TransportEvent::Request {
                process_id,
                request,
            } => {
                self.handle_client_request(process_id, request);
            }

            TransportEvent::Disconnected { process_id } => {
                if let Some(child) = self.children.get_mut(&process_id) {
                    child.disconnected_at = Some(Instant::now());
                }
            }
        }
    }

    fn handle_client_request(&mut self, process_id: ProcessId, request: ClientRequest) {
        match request {
            ClientRequest::CreateWindow {
                title,
                width,
                height,
                resizable,
            } => {
                let Some(child) = self.children.get_mut(&process_id) else {
                    return;
                };

                if child.registered_at.is_none() {
                    child.registered_at = Some(Instant::now());
                }

                child.disconnected_at = None;

                self.create_window_requests.push(CreateWindowRequest {
                    process_id,
                    content: child.content,
                    title,
                    width,
                    height,
                    resizable,
                });
            }

            ClientRequest::CloseWindow { window } => {
                let accepted = {
                    let Some(child) = self.children.get_mut(&process_id) else {
                        return;
                    };

                    if !child.windows.remove(&window) {
                        false
                    } else {
                        child.close_deadlines.remove(&window);

                        true
                    }
                };

                if accepted {
                    self.close_window_requests
                        .push(CloseWindowRequest { process_id, window });
                }
            }
        }
    }

    fn process_lifecycle(
        &mut self,
        active_processes: &HashSet<ProcessId>,
    ) -> Result<(), PlatformError> {
        let now = Instant::now();

        let process_ids: Vec<ProcessId> = self
            .children
            .iter()
            .filter_map(|(process_id, child)| {
                let registration_timeout = child.registered_at.is_none()
                    && now.duration_since(child.launched_at) >= REGISTRATION_TIMEOUT;

                let orphaned = child.registered_at.is_some_and(|registered_at| {
                    !active_processes.contains(process_id)
                        && now.duration_since(registered_at) >= ORPHAN_TIMEOUT
                });

                let disconnected = child.disconnected_at.is_some_and(|disconnected_at| {
                    now.duration_since(disconnected_at) >= DISCONNECT_TIMEOUT
                });

                let close_timeout = child
                    .close_deadlines
                    .values()
                    .any(|deadline| now >= *deadline);

                (registration_timeout || orphaned || disconnected || close_timeout)
                    .then_some(*process_id)
            })
            .collect();

        for process_id in process_ids {
            self.terminate_child(process_id)?;
        }

        Ok(())
    }

    fn spawn_managed_child(
        &mut self,
        content: WindowContent,
        bundle_id: String,
        mut command: Command,
    ) -> Result<ProcessId, PlatformError> {
        if let Some(process_id) = self
            .children
            .iter()
            .find_map(|(process_id, child)| (child.bundle_id == bundle_id).then_some(*process_id))
        {
            return Ok(process_id);
        }

        let child = command
            .env(BINDER_SOCKET_ENV, self.transport.socket_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| PlatformError::ProcessLaunchFailed)?;

        let process_id = ProcessId(child.id());

        self.children.insert(
            process_id,
            ManagedChild {
                content,
                bundle_id,

                child,

                launched_at: Instant::now(),
                registered_at: None,
                disconnected_at: None,

                windows: HashSet::new(),

                close_deadlines: HashMap::new(),
            },
        );

        Ok(process_id)
    }

    fn internal_app_for_entry(entry: &str) -> Option<InternalAppLaunch> {
        match entry {
            "internal:about" => Some(INTERNAL_ABOUT_APP),
            "internal:test" => Some(INTERNAL_TEST_APP),
            _ => None,
        }
    }

    fn internal_app_for_content(content: WindowContent) -> InternalAppLaunch {
        match content {
            WindowContent::About => INTERNAL_ABOUT_APP,
            WindowContent::Test => INTERNAL_TEST_APP,
        }
    }

    fn spawn_internal_app(
        &mut self,
        internal_app: InternalAppLaunch,
        bundle_id: String,
    ) -> Result<ProcessId, PlatformError> {
        let executable = std::env::current_exe().map_err(|_| PlatformError::ProcessLaunchFailed)?;

        let mut command = Command::new(executable);

        command.arg(internal_app.role_argument);

        self.spawn_managed_child(internal_app.content, bundle_id, command)
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPlatform for LinuxPlatform {
    fn system_bar_state(&self) -> Result<SystemBarState, PlatformError> {
        Ok(self.system_bar.clone())
    }

    fn open_system_settings(&self) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn perform_system_action(&self, _action: SystemAction) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn launch_internal_window(
        &mut self,
        content: WindowContent,
    ) -> Result<ProcessId, PlatformError> {
        let internal_app = Self::internal_app_for_content(content);

        self.spawn_internal_app(internal_app, String::from(internal_app.default_bundle_id))
    }

    fn register_window(
        &mut self,
        process_id: ProcessId,
        window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        {
            let child = self
                .children
                .get(&process_id)
                .ok_or(PlatformError::ServiceUnavailable)?;

            if child.registered_at.is_none() {
                return Err(PlatformError::InvalidResponse);
            }
        }

        self.transport
            .send_event(process_id, ServerEvent::WindowCreated { window })?;

        if let Some(child) = self.children.get_mut(&process_id) {
            child.windows.insert(window);
        }

        Ok(())
    }

    fn request_window_close(
        &mut self,
        process_id: ProcessId,
        window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        {
            let child = self
                .children
                .get(&process_id)
                .ok_or(PlatformError::ServiceUnavailable)?;

            if !child.windows.contains(&window) {
                return Err(PlatformError::PermissionDenied);
            }

            if child.close_deadlines.contains_key(&window) {
                return Ok(());
            }
        }

        self.transport
            .send_event(process_id, ServerEvent::CloseRequested { window })?;

        if let Some(child) = self.children.get_mut(&process_id) {
            child
                .close_deadlines
                .insert(window, Instant::now() + CLOSE_TIMEOUT);
        }

        Ok(())
    }

    fn notify_window_resized(
        &mut self,
        notification: WindowResizedNotification,
    ) -> Result<(), PlatformError> {
        {
            let child = self
                .children
                .get(&notification.process_id)
                .ok_or(PlatformError::ServiceUnavailable)?;

            if !child.windows.contains(&notification.window) {
                return Err(PlatformError::PermissionDenied);
            }
        }

        self.transport.send_event(
            notification.process_id,
            ServerEvent::Resized {
                window: notification.window,

                width: notification.width,

                height: notification.height,
            },
        )
    }

    fn notify_window_focus_changed(
        &mut self,
        notification: WindowFocusChangedNotification,
    ) -> Result<(), PlatformError> {
        {
            let child = self
                .children
                .get(&notification.process_id)
                .ok_or(PlatformError::ServiceUnavailable)?;

            if !child.windows.contains(&notification.window) {
                return Err(PlatformError::PermissionDenied);
            }
        }

        self.transport.send_event(
            notification.process_id,
            ServerEvent::FocusChanged {
                window: notification.window,

                focused: notification.focused,
            },
        )
    }

    fn synchronize_applications(
        &mut self,
        active_processes: &[ProcessId],
    ) -> Result<(), PlatformError> {
        let active_processes: HashSet<ProcessId> = active_processes.iter().copied().collect();

        self.process_lifecycle(&active_processes)
    }

    fn take_create_window_requests(&mut self) -> Vec<CreateWindowRequest> {
        std::mem::take(&mut self.create_window_requests)
    }

    fn take_close_window_requests(&mut self) -> Vec<CloseWindowRequest> {
        std::mem::take(&mut self.close_window_requests)
    }

    fn take_exited_processes(&mut self) -> Vec<ProcessId> {
        std::mem::take(&mut self.exited_processes)
    }

    fn refresh(&mut self) -> Result<bool, PlatformError> {
        self.reap_exited_children()?;

        let allowed_processes: HashSet<ProcessId> = self.children.keys().copied().collect();

        let events = self.transport.poll(&allowed_processes)?;

        for event in events {
            self.handle_transport_event(event);
        }

        let next = read_system_bar_state()?;

        let changed = next != self.system_bar;

        if changed {
            self.system_bar = next;
        }

        Ok(changed)
    }

    fn get_apps(&self) -> Vec<AppInfo> {
        read_apps()
    }

    fn launch_app(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        if let Some(internal_app) = Self::internal_app_for_entry(&app.entry) {
            return self.spawn_internal_app(internal_app, app.bundle_id.clone());
        }

        let executable = app.entry_path();

        if !executable.is_file() {
            return Err(PlatformError::ProcessLaunchFailed);
        }

        Err(PlatformError::UnsupportedOperation)
    }

    fn running_app_bundle_ids(&self) -> Vec<String> {
        self.children
            .values()
            .map(|child| child.bundle_id.clone())
            .collect()
    }
}

impl Drop for LinuxPlatform {
    fn drop(&mut self) {
        for (_process_id, mut child) in self.children.drain() {
            let _ = child.child.kill();

            let _ = child.child.wait();
        }
    }
}

pub(super) fn run_internal_process(content: WindowContent) -> Result<(), PlatformError> {
    transport::run_internal_process(content)
}

fn read_system_bar_state() -> Result<SystemBarState, PlatformError> {
    Ok(SystemBarState {
        clock: read_clock()?,

        ..SystemBarState::default()
    })
}

fn read_clock() -> Result<ClockState, PlatformError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PlatformError::InvalidResponse)?;

    let timestamp: libc::time_t = duration
        .as_secs()
        .try_into()
        .map_err(|_| PlatformError::InvalidResponse)?;

    let mut local_time = std::mem::MaybeUninit::<libc::tm>::uninit();

    let result = unsafe { libc::localtime_r(&timestamp, local_time.as_mut_ptr()) };

    if result.is_null() {
        return Err(PlatformError::TransportFailure);
    }

    let local_time = unsafe { local_time.assume_init() };

    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    let weekday = weekdays
        .get(local_time.tm_wday as usize)
        .copied()
        .unwrap_or("");

    Ok(ClockState {
        date: format!(
            "{:02}/{:02} {}",
            local_time.tm_mon + 1,
            local_time.tm_mday,
            weekday,
        ),

        time: format!("{:02}:{:02}", local_time.tm_hour, local_time.tm_min,),
    })
}

fn resources_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")
}

fn applications_root() -> PathBuf {
    resources_root().join("apps")
}

fn read_apps() -> Vec<AppInfo> {
    let root = applications_root();

    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut apps = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
            continue;
        }

        if !path.is_dir() {
            continue;
        }

        if let Some(app) = read_app_manifest(&path) {
            apps.push(app);
        }
    }

    apps.sort_by(|left, right| left.name.cmp(&right.name));

    apps
}

fn read_app_manifest(app_root: &Path) -> Option<AppInfo> {
    let manifest_path = app_root.join("about.toml");

    let content = fs::read_to_string(manifest_path).ok()?;

    let name = parse_string_field(&content, "name")?;

    let bundle_id = parse_string_field(&content, "bundle_id")?;

    let entry = parse_string_field(&content, "entry")?;

    let version = parse_string_field(&content, "version").unwrap_or_default();

    let developer = parse_string_field(&content, "developer").unwrap_or_default();

    let description = parse_string_field(&content, "description").unwrap_or_default();

    let icon = parse_string_field(&content, "icon").map(|path| resolve_app_path(app_root, &path));

    let resources = parse_string_array_field(&content, "resources")
        .into_iter()
        .map(|path| resolve_app_path(app_root, &path))
        .collect();

    Some(AppInfo {
        root: app_root.to_path_buf(),

        name,
        bundle_id,
        version,
        developer,
        entry,
        description,
        icon,
        resources,
    })
}

fn resolve_app_path(app_root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);

    if path.is_absolute() {
        path
    } else {
        app_root.join(path)
    }
}

fn parse_string_field(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();

        let Some((field, value)) = line.split_once('=') else {
            continue;
        };

        if field.trim() != key {
            continue;
        }

        return parse_string_literals(value).into_iter().next();
    }

    None
}

fn parse_string_array_field(content: &str, key: &str) -> Vec<String> {
    let mut values = Vec::new();

    let mut in_array = false;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if in_array {
            values.extend(parse_string_literals(line));

            if line.contains(']') {
                in_array = false;
            }

            continue;
        }

        let Some((field, value)) = line.split_once('=') else {
            continue;
        };

        if field.trim() != key {
            continue;
        }

        values.extend(parse_string_literals(value));

        if value.contains('[') && !value.contains(']') {
            in_array = true;
        }
    }

    values
}

fn parse_string_literals(text: &str) -> Vec<String> {
    let mut values = Vec::new();

    let mut current = String::new();

    let mut in_string = false;

    let mut escaped = false;

    for character in text.chars() {
        if !in_string {
            if character == '"' {
                in_string = true;
                current.clear();
            }

            continue;
        }

        if escaped {
            current.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });

            escaped = false;

            continue;
        }

        if character == '\\' {
            escaped = true;

            continue;
        }

        if character == '"' {
            values.push(current.clone());

            current.clear();

            in_string = false;

            continue;
        }

        current.push(character);
    }

    values
}
