use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "mochios"))]
use std::process::{Command, Stdio};

use crate::apps;

#[cfg(target_os = "mochios")]
mod context_menu;
#[cfg(target_os = "mochios")]
mod decoration;

use super::{
    AppInfo, ClockState, CloseWindowRequest, ContextMenuModel, CreateWindowRequest,
    DesktopPlatform, PlatformError, ProcessId, RemoteWindowId, SystemAction, SystemBarState,
};
use viewkit::prelude::State;

#[derive(Debug)]
struct ManagedApp {
    bundle_id: String,
    child: Option<Child>,
    windows: HashSet<RemoteWindowId>,
    track_kernel_lifecycle: bool,
    linux_instance: Option<u64>,
}

const PROCESS_RECORD_SIZE: usize = 88;
#[cfg(target_os = "mochios")]
const MAX_PROCESS_RECORDS: usize = 256;
const PROCESS_STATE_TERMINATED: u64 = 4;
#[cfg(target_os = "mochios")]
const LINUX_SERVICE_NAME: &str = "linux.service";

#[allow(unused)]
pub struct MochiOsPlatform {
    system_bar: SystemBarState,
    apps: Vec<AppInfo>,
    children: HashMap<ProcessId, ManagedApp>,
    create_window_requests: Vec<CreateWindowRequest>,
    close_window_requests: Vec<CloseWindowRequest>,
    exited_processes: Vec<ProcessId>,
    next_internal_pid: u32,
    next_app_scan: Option<Instant>,
    #[cfg(target_os = "mochios")]
    decoration_manager: Option<decoration::DecorationManager>,
    #[cfg(target_os = "mochios")]
    decoration_connect_attempted: bool,
    context_menu_state: State<Option<ContextMenuModel>>,
    #[cfg(target_os = "mochios")]
    context_menu_manager: Option<context_menu::ContextMenuManager>,
    #[cfg(target_os = "mochios")]
    context_menu_connect_attempted: bool,
    #[cfg(target_os = "mochios")]
    _desktop_background: Option<viewkit::platform::mochios::DesktopBackground>,
}

impl MochiOsPlatform {
    #[allow(unused)]
    pub fn new() -> Self {
        Self::with_context_menu_state(State::new(None))
    }

    pub fn with_context_menu_state(context_menu_state: State<Option<ContextMenuModel>>) -> Self {
        Self {
            system_bar: SystemBarState::default(),
            apps: read_apps(),
            children: HashMap::new(),
            create_window_requests: Vec::new(),
            close_window_requests: Vec::new(),
            exited_processes: Vec::new(),
            next_internal_pid: 0x4000_0000,
            next_app_scan: None,
            #[cfg(target_os = "mochios")]
            decoration_manager: None,
            #[cfg(target_os = "mochios")]
            decoration_connect_attempted: false,
            context_menu_state,
            #[cfg(target_os = "mochios")]
            context_menu_manager: None,
            #[cfg(target_os = "mochios")]
            context_menu_connect_attempted: false,
            #[cfg(target_os = "mochios")]
            _desktop_background: crate::ui::wallpaper::Wallpaper::load_default_image().and_then(
                |image| viewkit::platform::mochios::DesktopBackground::from_image(image).ok(),
            ),
        }
    }

    fn replace_apps_if_changed(&mut self, discovered: Vec<AppInfo>) -> bool {
        if self.apps == discovered {
            return false;
        }
        self.apps = discovered;
        true
    }

    fn next_process_id(&mut self) -> ProcessId {
        let process_id = ProcessId(self.next_internal_pid);
        self.next_internal_pid = self.next_internal_pid.saturating_add(1).max(0x4000_0000);
        process_id
    }

    fn launch_internal_renderer(
        &mut self,
        entry: &str,
        bundle_id: String,
    ) -> Result<ProcessId, PlatformError> {
        if let Some(process_id) = self.process_for_bundle(&bundle_id) {
            return Ok(process_id);
        }

        let process_id = self.next_process_id();
        self.children.insert(
            process_id,
            ManagedApp {
                bundle_id,
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: false,
                linux_instance: None,
            },
        );

        for request in create_internal_window_requests(process_id, entry) {
            self.create_window_requests.push(request);
        }

        Ok(process_id)
    }

    fn spawn_entry(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        if let Some(process_id) = self.process_for_bundle(&app.bundle_id) {
            return Ok(process_id);
        }

        let (process_id, child) = spawn_application(app)?;
        self.children.insert(
            process_id,
            ManagedApp {
                bundle_id: app.bundle_id.clone(),
                child,
                windows: HashSet::new(),
                track_kernel_lifecycle: true,
                linux_instance: None,
            },
        );

        Ok(process_id)
    }

    fn process_for_bundle(&self, bundle_id: &str) -> Option<ProcessId> {
        self.children
            .iter()
            .find_map(|(process_id, child)| (child.bundle_id == bundle_id).then_some(*process_id))
    }

    fn reap_exited_children(&mut self) -> Result<bool, PlatformError> {
        let mut exited = Vec::new();

        for (process_id, managed) in &mut self.children {
            let Some(child) = managed.child.as_mut() else {
                continue;
            };

            match child.try_wait() {
                Ok(Some(_)) => exited.push(*process_id),
                Ok(None) => {}
                Err(_) => return Err(PlatformError::ProcessTerminationFailed),
            }
        }

        let mut changed = !exited.is_empty();
        for process_id in exited {
            self.children.remove(&process_id);
            self.exited_processes.push(process_id);
        }

        #[cfg(target_os = "mochios")]
        {
            let linux_instances: Vec<(ProcessId, u64)> = self
                .children
                .iter()
                .filter_map(|(process_id, managed)| {
                    managed
                        .linux_instance
                        .map(|instance| (*process_id, instance))
                })
                .collect();
            let mut retained_linux = HashSet::new();
            for (_, instance) in linux_instances {
                match linux_application_is_running(instance) {
                    Ok(true) => {
                        retained_linux.insert(instance);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        retained_linux.insert(instance);
                        eprintln!(
                            "failed to query Linux application instance {instance}: {error:?}"
                        );
                    }
                }
            }
            changed = self.remove_exited_linux_processes(&retained_linux) || changed;

            if self
                .children
                .values()
                .any(|managed| managed.track_kernel_lifecycle)
            {
                let live_processes = inspect_live_processes()?;
                changed = self.remove_exited_kernel_processes(&live_processes) || changed;
            }
        }

        Ok(changed)
    }

    fn remove_exited_kernel_processes(&mut self, live_processes: &HashSet<ProcessId>) -> bool {
        let exited: Vec<ProcessId> = self
            .children
            .iter()
            .filter_map(|(process_id, managed)| {
                (managed.track_kernel_lifecycle && !live_processes.contains(process_id))
                    .then_some(*process_id)
            })
            .collect();
        let changed = !exited.is_empty();
        for process_id in exited {
            self.children.remove(&process_id);
            self.exited_processes.push(process_id);
        }
        changed
    }

    fn remove_exited_linux_processes(&mut self, running_instances: &HashSet<u64>) -> bool {
        let exited: Vec<ProcessId> = self
            .children
            .iter()
            .filter_map(|(process_id, managed)| {
                managed
                    .linux_instance
                    .filter(|instance| !running_instances.contains(instance))
                    .map(|_| *process_id)
            })
            .collect();
        let changed = !exited.is_empty();
        for process_id in exited {
            self.children.remove(&process_id);
            self.exited_processes.push(process_id);
        }
        changed
    }

    #[cfg(target_os = "mochios")]
    fn launch_linux_bundle(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        if let Some(process_id) = self.process_for_bundle(&app.bundle_id) {
            return Ok(process_id);
        }
        let declared = app
            .entry
            .strip_prefix("linux:")
            .filter(|declared| *declared == app.bundle_id)
            .ok_or(PlatformError::ProcessLaunchFailed)?;
        let user = std::env::var("USER")
            .ok()
            .filter(|user| !user.is_empty())
            .ok_or(PlatformError::ProcessLaunchFailed)?;
        let instance = launch_linux_bundle(declared, &user)?;
        let process_id = self.next_process_id();
        self.children.insert(
            process_id,
            ManagedApp {
                bundle_id: app.bundle_id.clone(),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: false,
                linux_instance: Some(instance),
            },
        );
        Ok(process_id)
    }
}

fn decode_live_process_ids(buffer: &[u8], record_count: usize) -> HashSet<ProcessId> {
    buffer
        .chunks_exact(PROCESS_RECORD_SIZE)
        .take(record_count)
        .filter_map(|record| {
            let pid = u64::from_ne_bytes(record.get(0..8)?.try_into().ok()?);
            let state = u64::from_ne_bytes(record.get(16..24)?.try_into().ok()?);
            (pid != 0 && pid <= u32::MAX as u64 && state != PROCESS_STATE_TERMINATED)
                .then_some(ProcessId(pid as u32))
        })
        .collect()
}

#[cfg(target_os = "mochios")]
fn inspect_live_processes() -> Result<HashSet<ProcessId>, PlatformError> {
    use mochi_user_syscall as syscall;

    let mut records = vec![0u8; PROCESS_RECORD_SIZE * MAX_PROCESS_RECORDS];
    let count = syscall::call2(
        syscall::SyscallNumber::ListProcesses,
        records.as_mut_ptr() as u64,
        records.len() as u64,
    )
    .map_err(|_| PlatformError::ProcessTerminationFailed)?;
    let count = usize::try_from(count)
        .unwrap_or(MAX_PROCESS_RECORDS)
        .min(MAX_PROCESS_RECORDS);
    Ok(decode_live_process_ids(&records, count))
}

#[cfg(target_os = "mochios")]
fn spawn_application(app: &AppInfo) -> Result<(ProcessId, Option<Child>), PlatformError> {
    const CAPABILITY_SERVICE_NAME: &str = "capability.service";
    use mochi_user_syscall as syscall;

    let executable = app.entry_path();
    let executable = executable
        .to_str()
        .ok_or(PlatformError::ProcessLaunchFailed)?;
    let environment = session_environment_arguments();
    let request = encode_spawn_app_request_with_environment(
        executable,
        shell_endpoint_from_environment(),
        prompt_is_interactive(),
        &environment,
    )?;
    let endpoint = syscall::call2(
        syscall::SyscallNumber::FindProcessByName,
        CAPABILITY_SERVICE_NAME.as_ptr() as u64,
        CAPABILITY_SERVICE_NAME.len() as u64,
    )
    .map_err(|error| {
        eprintln!("failed to find capability.service: {error:?}");
        PlatformError::ProcessLaunchFailed
    })?;
    if endpoint == 0 {
        eprintln!("failed to find capability.service: invalid endpoint");
        return Err(PlatformError::ProcessLaunchFailed);
    }

    let mut reply = [0u8; 16];
    let message = syscall::call5(
        syscall::SyscallNumber::IpcCall,
        endpoint,
        request.as_ptr() as u64,
        request.len() as u64,
        reply.as_mut_ptr() as u64,
        reply.len() as u64,
    )
    .map_err(|error| {
        eprintln!("capability.service app launch request failed: {error:?}");
        PlatformError::ProcessLaunchFailed
    })?;
    let reply_len = (message & 0xffff_ffff) as usize;
    if reply_len < reply.len() {
        eprintln!("capability.service returned a short app launch reply");
        return Err(PlatformError::ProcessLaunchFailed);
    }

    let status = u64::from_le_bytes(
        reply[..8]
            .try_into()
            .map_err(|_| PlatformError::InvalidResponse)?,
    );
    if status != 0 {
        eprintln!("capability.service rejected app launch: errno={status}");
        return Err(PlatformError::ProcessLaunchRejected { errno: status });
    }
    let pid = u64::from_le_bytes(
        reply[8..]
            .try_into()
            .map_err(|_| PlatformError::InvalidResponse)?,
    );
    if pid == 0 || pid > u32::MAX as u64 {
        eprintln!("capability.service returned an invalid app pid");
        return Err(PlatformError::InvalidResponse);
    }

    Ok((ProcessId(pid as u32), None))
}

#[cfg(not(target_os = "mochios"))]
fn spawn_application(app: &AppInfo) -> Result<(ProcessId, Option<Child>), PlatformError> {
    let executable = app.entry_path();
    if !executable.is_file() {
        return Err(PlatformError::ProcessLaunchFailed);
    }

    let child = Command::new(&executable)
        .env("MOCHI_EXECUTABLE_PATH", executable.as_os_str())
        .env("MOCHI_APP_BUNDLE_PATH", app.root.as_os_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|_| PlatformError::ProcessLaunchFailed)?;
    Ok((ProcessId(child.id()), Some(child)))
}

const SPAWN_APP_OPCODE: u32 = 0x4150_5053;
const SPAWN_APP_HEADER_LEN: usize = 24;
const EXEC_MANIFEST_ENV_PREFIX: &str = "__MNU_EXEC_ENV=";
const SESSION_ENVIRONMENT_NAMES: [&str; 4] = ["HOME", "USER", "LOGNAME", "SHELL"];

fn encode_spawn_app_request(
    executable: &str,
    shell_endpoint: u64,
    interactive: bool,
) -> Result<Vec<u8>, PlatformError> {
    encode_spawn_app_request_with_environment(executable, shell_endpoint, interactive, &[])
}

fn encode_spawn_app_request_with_environment(
    executable: &str,
    shell_endpoint: u64,
    interactive: bool,
    environment: &[String],
) -> Result<Vec<u8>, PlatformError> {
    if executable.is_empty() || !executable.starts_with('/') || executable.as_bytes().contains(&0) {
        return Err(PlatformError::ProcessLaunchFailed);
    }
    if environment.iter().any(|item| item.as_bytes().contains(&0)) {
        return Err(PlatformError::ProcessLaunchFailed);
    }

    let payload_len =
        executable.len() + 1 + environment.iter().map(|item| item.len() + 1).sum::<usize>();
    let mut request = vec![0u8; SPAWN_APP_HEADER_LEN + payload_len];
    request[0..4].copy_from_slice(&SPAWN_APP_OPCODE.to_le_bytes());
    request[8..16].copy_from_slice(&shell_endpoint.to_le_bytes());
    request[16] = u8::from(interactive);
    let mut cursor = SPAWN_APP_HEADER_LEN;
    request[cursor..cursor + executable.len()].copy_from_slice(executable.as_bytes());
    cursor += executable.len() + 1;
    for item in environment {
        request[cursor..cursor + item.len()].copy_from_slice(item.as_bytes());
        cursor += item.len() + 1;
    }
    Ok(request)
}

fn session_environment_arguments() -> Vec<String> {
    SESSION_ENVIRONMENT_NAMES
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{EXEC_MANIFEST_ENV_PREFIX}{name}={value}"))
        })
        .collect()
}

#[cfg(target_os = "mochios")]
fn shell_endpoint_from_environment() -> u64 {
    std::env::var("MOCHI_SHELL_ENDPOINT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "mochios")]
fn prompt_is_interactive() -> bool {
    std::env::var("MOCHI_PROMPT_MODE").as_deref() == Ok("interactive")
}

impl Default for MochiOsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPlatform for MochiOsPlatform {
    fn system_bar_state(&self) -> Result<SystemBarState, PlatformError> {
        Ok(self.system_bar.clone())
    }

    fn native_windows_overlap(&self, area: viewkit::prelude::Rect) -> Result<bool, PlatformError> {
        #[cfg(target_os = "mochios")]
        {
            self.decoration_manager
                .as_ref()
                .ok_or(PlatformError::ServiceUnavailable)?
                .windows_overlap(area)
                .map_err(|_| PlatformError::TransportFailure)
        }
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = area;
            Ok(false)
        }
    }

    fn reload_appearance(&mut self) -> Result<bool, PlatformError> {
        #[cfg(target_os = "mochios")]
        {
            self._desktop_background = crate::ui::wallpaper::Wallpaper::load_default_image()
                .and_then(|image| {
                    viewkit::platform::mochios::DesktopBackground::from_image(image).ok()
                });
            return Ok(true);
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Ok(false)
        }
    }

    fn open_system_settings(&self) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn perform_system_action(&self, action: SystemAction) -> Result<(), PlatformError> {
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = action;
            return Err(PlatformError::UnsupportedOperation);
        }
        #[cfg(target_os = "mochios")]
        match action {
            SystemAction::LockScreen => {
                request_session_action(mochi_user_platform::session_control::Action::Lock)
            }
            SystemAction::LogOut => {
                request_session_action(mochi_user_platform::session_control::Action::LogOut)?;
                viewkit::request_exit();
                Ok(())
            }
            SystemAction::Sleep | SystemAction::Restart | SystemAction::ShutDown => {
                Err(PlatformError::UnsupportedOperation)
            }
        }
    }

    fn launch_internal_window(&mut self, entry: &str) -> Result<ProcessId, PlatformError> {
        self.launch_internal_renderer(entry, String::from(internal_bundle_id(entry)))
    }

    fn register_window(
        &mut self,
        process_id: ProcessId,
        window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        let child = self
            .children
            .get_mut(&process_id)
            .ok_or(PlatformError::ServiceUnavailable)?;
        child.windows.insert(window);
        Ok(())
    }

    fn request_window_close(
        &mut self,
        process_id: ProcessId,
        window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        let Some(child) = self.children.get_mut(&process_id) else {
            return Ok(());
        };

        if !child.windows.remove(&window) {
            return Ok(());
        }

        self.close_window_requests
            .push(CloseWindowRequest { process_id, window });
        if child.child.is_none() && child.windows.is_empty() {
            self.children.remove(&process_id);
            self.exited_processes.push(process_id);
        }
        Ok(())
    }

    fn synchronize_applications(
        &mut self,
        _active_processes: &[ProcessId],
    ) -> Result<(), PlatformError> {
        let now = Instant::now();
        if self.next_app_scan.is_some_and(|next| now < next) {
            return Ok(());
        }
        self.next_app_scan = now.checked_add(Duration::from_secs(1));
        self.replace_apps_if_changed(read_apps());
        Ok(())
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

    fn handle_platform_message(&mut self, message: &[u8]) -> bool {
        #[cfg(target_os = "mochios")]
        if let Some(manager) = self.context_menu_manager.as_mut() {
            match manager.handle_message(message, &self.context_menu_state) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    eprintln!("Binder context menu manager failed: {error}");
                    return false;
                }
            }
        }
        #[cfg(target_os = "mochios")]
        if let Some(manager) = self.decoration_manager.as_mut() {
            return match manager.handle_message(message) {
                Ok(handled) => handled,
                Err(error) => {
                    eprintln!("Binder decoration manager failed: {error}");
                    false
                }
            };
        }
        #[cfg(not(target_os = "mochios"))]
        let _ = message;
        false
    }

    fn refresh(&mut self) -> Result<bool, PlatformError> {
        // TODO
        // time.service
        // notification.service
        // network.service
        // audio.service
        // power.service
        //
        // から状態を取得またはイベントを受信する。
        #[cfg(target_os = "mochios")]
        if !self.decoration_connect_attempted {
            self.decoration_connect_attempted = true;
            match decoration::DecorationManager::connect() {
                Ok(manager) => self.decoration_manager = Some(manager),
                Err(error) => eprintln!("Binder decoration manager unavailable: {error}"),
            }
        }
        #[cfg(target_os = "mochios")]
        if !self.context_menu_connect_attempted {
            self.context_menu_connect_attempted = true;
            match context_menu::ContextMenuManager::connect() {
                Ok(manager) => self.context_menu_manager = Some(manager),
                Err(error) => eprintln!("Binder context menu manager unavailable: {error}"),
            }
        }

        let children_changed = self.reap_exited_children()?;
        let clock = read_clock()?;
        let clock_changed = self.system_bar.clock != clock;
        if clock_changed {
            self.system_bar.clock = clock;
        }

        Ok(children_changed || clock_changed)
    }

    fn get_apps(&self) -> Vec<AppInfo> {
        self.apps.clone()
    }

    fn launch_app(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        match app.entry.as_str() {
            #[cfg(target_os = "mochios")]
            entry if entry.starts_with("linux:") => self.launch_linux_bundle(app),
            apps::ABOUT_ENTRY | apps::TEST_ENTRY => {
                self.launch_internal_renderer(&app.entry, app.bundle_id.clone())
            }
            _ => self.spawn_entry(app),
        }
    }

    fn process_id_for_bundle(&self, bundle_id: &str) -> Option<ProcessId> {
        self.process_for_bundle(bundle_id)
    }

    fn activate_application(&self, process_id: ProcessId) -> Result<(), PlatformError> {
        #[cfg(target_os = "mochios")]
        {
            mochi_user_platform::workspace::activate_application(u64::from(process_id.0))
                .map_err(|_| PlatformError::TransportFailure)
        }
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = process_id;
            Ok(())
        }
    }

    fn running_app_bundle_ids(&self) -> Vec<String> {
        self.children
            .values()
            .map(|child| child.bundle_id.clone())
            .collect()
    }

    fn complete_context_menu(
        &mut self,
        request_id: u64,
        command_id: Option<u32>,
    ) -> Result<(), PlatformError> {
        #[cfg(target_os = "mochios")]
        {
            let manager = self
                .context_menu_manager
                .as_mut()
                .ok_or(PlatformError::ServiceUnavailable)?;
            return manager
                .complete(request_id, command_id)
                .map_err(|_| PlatformError::TransportFailure);
        }
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = (request_id, command_id);
            Err(PlatformError::UnsupportedOperation)
        }
    }
}

#[cfg(target_os = "mochios")]
fn request_session_action(
    action: mochi_user_platform::session_control::Action,
) -> Result<(), PlatformError> {
    use mochi_user_platform::session_control::{
        RESPONSE_LEN, Request, decode_response, encode_request,
    };
    use mochi_user_syscall as syscall;

    const SERVICE_MANAGER_NAME: &str = "service-manager.service";
    let session_id = std::env::var("MOCHI_SESSION_ID")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value != 0)
        .ok_or(PlatformError::InvalidResponse)?;
    let endpoint = syscall::call2(
        syscall::SyscallNumber::FindProcessByName,
        SERVICE_MANAGER_NAME.as_ptr() as u64,
        SERVICE_MANAGER_NAME.len() as u64,
    )
    .map_err(|_| PlatformError::ServiceUnavailable)?;
    let request = encode_request(Request { action, session_id });
    let mut response = [0u8; RESPONSE_LEN];
    let received = syscall::call5(
        syscall::SyscallNumber::IpcCall,
        endpoint,
        request.as_ptr() as u64,
        request.len() as u64,
        response.as_mut_ptr() as u64,
        response.len() as u64,
    )
    .map_err(|_| PlatformError::TransportFailure)?;
    if (received & 0xffff_ffff) as usize != RESPONSE_LEN {
        return Err(PlatformError::InvalidResponse);
    }
    let response = decode_response(&response).map_err(|_| PlatformError::InvalidResponse)?;
    if response.action != action || response.session_id != session_id {
        return Err(PlatformError::InvalidResponse);
    }
    if response.status != 0 {
        return Err(PlatformError::PermissionDenied);
    }
    Ok(())
}

fn read_clock() -> Result<ClockState, PlatformError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PlatformError::InvalidResponse)?
        .as_secs()
        .try_into()
        .map_err(|_| PlatformError::InvalidResponse)?;

    clock_from_unix_seconds(seconds)
}

fn clock_from_unix_seconds(seconds: i64) -> Result<ClockState, PlatformError> {
    let days = seconds.div_euclid(86_400);
    let seconds_in_day = seconds.rem_euclid(86_400);
    let hour = seconds_in_day / 3_600;
    let minute = (seconds_in_day % 3_600) / 60;

    let shifted_days = days
        .checked_add(719_468)
        .ok_or(PlatformError::InvalidResponse)?;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_phase = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_phase + 2) / 5 + 1;
    let month = month_phase + if month_phase < 10 { 3 } else { -9 };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(PlatformError::InvalidResponse);
    }

    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let weekday = weekdays[(days + 4).rem_euclid(7) as usize];

    Ok(ClockState {
        date: format!("{month:02}/{day:02} {weekday}"),
        time: format!("{hour:02}:{minute:02}"),
    })
}

#[allow(unused)]
pub(super) fn run_internal_process(entry: &str) -> Result<(), PlatformError> {
    match entry {
        apps::ABOUT_ENTRY => {
            viewkit::run::<AboutProcessApp>().map_err(|_| PlatformError::ProcessLaunchFailed)
        }
        apps::TEST_ENTRY => {
            viewkit::run::<TestProcessApp>().map_err(|_| PlatformError::ProcessLaunchFailed)
        }
        _ => Err(PlatformError::UnsupportedOperation),
    }
}

struct AboutProcessApp;

impl viewkit::prelude::App for AboutProcessApp {
    type Body = Box<dyn viewkit::prelude::View + 'static>;

    fn new() -> Self {
        Self
    }

    fn window(&self) -> viewkit::prelude::WindowOptions {
        viewkit::prelude::WindowOptions::new("About mochiOS")
            .size(420.0, 300.0)
            .resizable(true)
    }

    fn body(&self, _context: &viewkit::prelude::ViewContext) -> Self::Body {
        Box::new(crate::ui::about::view())
    }
}

struct TestProcessApp;

impl viewkit::prelude::App for TestProcessApp {
    type Body = Box<dyn viewkit::prelude::View + 'static>;

    fn new() -> Self {
        Self
    }

    fn window(&self) -> viewkit::prelude::WindowOptions {
        viewkit::prelude::WindowOptions::new("Test Window")
            .size(360.0, 220.0)
            .resizable(true)
    }

    fn body(&self, _context: &viewkit::prelude::ViewContext) -> Self::Body {
        Box::new(crate::ui::test::view(Default::default()))
    }
}

fn create_internal_window_requests(process_id: ProcessId, entry: &str) -> Vec<CreateWindowRequest> {
    match entry {
        apps::ABOUT_ENTRY => vec![CreateWindowRequest {
            process_id,
            renderer: String::from(apps::ABOUT_ENTRY),
            title: String::from("About mochiOS"),
            width: 420,
            height: 300,
            resizable: true,
        }],
        apps::TEST_ENTRY => (1..=3)
            .map(|index| CreateWindowRequest {
                process_id,
                renderer: String::from(apps::TEST_ENTRY),
                title: format!("Test Window {index}"),
                width: 360,
                height: 220,
                resizable: true,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn internal_bundle_id(entry: &str) -> &'static str {
    match entry {
        apps::ABOUT_ENTRY => apps::ABOUT_BUNDLE_ID,
        apps::TEST_ENTRY => apps::TEST_BUNDLE_ID,
        _ => "org.mochios.binder.internal",
    }
}

fn applications_root() -> PathBuf {
    PathBuf::from("/applications")
}

fn read_apps() -> Vec<AppInfo> {
    let mut apps = read_apps_from(&applications_root());
    apps.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.bundle_id.cmp(&right.bundle_id))
    });
    apps
}

#[cfg(target_os = "mochios")]
fn launch_linux_bundle(bundle_id: &str, user: &str) -> Result<u64, PlatformError> {
    use mochios_linux_gui_protocol::{
        BundleLaunchRequest, BundleLaunchResponse, LAUNCH_RESPONSE_LEN, MAX_BUNDLE_ID_LEN,
    };

    let service = mochi_user_platform::process::find_by_name(LINUX_SERVICE_NAME)
        .map_err(|_| PlatformError::ServiceUnavailable)?;
    if service == 0 || bundle_id.len() > MAX_BUNDLE_ID_LEN {
        return Err(PlatformError::ServiceUnavailable);
    }
    let request_id = mochi_user_platform::time::ticks().unwrap_or(1).max(1);
    let request = BundleLaunchRequest {
        request_id,
        bundle_id,
        user,
    };
    let mut encoded = [0u8; 256];
    let length = request
        .encode(&mut encoded)
        .map_err(|_| PlatformError::InvalidResponse)?;
    let mut response = [0u8; LAUNCH_RESPONSE_LEN];
    let raw = mochi_user_platform::ipc::call(service, &encoded[..length], &mut response)
        .map_err(|_| PlatformError::TransportFailure)?;
    let length = raw as u32 as usize;
    let response = BundleLaunchResponse::decode(
        response
            .get(..length)
            .ok_or(PlatformError::InvalidResponse)?,
    )
    .map_err(|_| PlatformError::InvalidResponse)?;
    if response.request_id != request_id || response.status != 0 || response.instance == 0 {
        return Err(PlatformError::ProcessLaunchFailed);
    }
    Ok(response.instance)
}

#[cfg(target_os = "mochios")]
fn linux_application_is_running(instance: u64) -> Result<bool, PlatformError> {
    use mochios_linux_gui_protocol::{
        STATUS_REQUEST_LEN, STATUS_RESPONSE_LEN, StatusRequest, StatusResponse,
    };

    let service = mochi_user_platform::process::find_by_name(LINUX_SERVICE_NAME)
        .map_err(|_| PlatformError::ServiceUnavailable)?;
    if service == 0 {
        return Err(PlatformError::ServiceUnavailable);
    }
    let request_id = mochi_user_platform::time::ticks()
        .unwrap_or(1)
        .wrapping_add(instance.rotate_left(17))
        .max(1);
    let request = StatusRequest {
        request_id,
        instance,
    };
    let mut encoded = [0u8; STATUS_REQUEST_LEN];
    request
        .encode(&mut encoded)
        .map_err(|_| PlatformError::InvalidResponse)?;
    let mut response = [0u8; STATUS_RESPONSE_LEN];
    let raw = mochi_user_platform::ipc::call(service, &encoded, &mut response)
        .map_err(|_| PlatformError::TransportFailure)?;
    let length = raw as u32 as usize;
    let response = StatusResponse::decode(
        response
            .get(..length)
            .ok_or(PlatformError::InvalidResponse)?,
    )
    .map_err(|_| PlatformError::InvalidResponse)?;
    if response.request_id != request_id || response.instance != instance {
        return Err(PlatformError::InvalidResponse);
    }
    if response.status != 0 {
        return Err(PlatformError::ProcessLaunchRejected {
            errno: u64::from(response.status.unsigned_abs()),
        });
    }
    Ok(response.running)
}

fn read_apps_from(root: &Path) -> Vec<AppInfo> {
    let Ok(entries) = read_directory_entries(root) else {
        return Vec::new();
    };

    let mut apps = Vec::new();

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("Binder.app") {
            continue;
        }
        if let Some(app) = read_app_about(&path)
            && app.bundle_id != "org.mochios.binder"
        {
            apps.push(app);
        }
    }

    apps.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.bundle_id.cmp(&right.bundle_id))
    });
    apps
}

fn read_directory_entries(root: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut last_error = None;
    for _ in 0..8 {
        match fs::read_dir(root) {
            Ok(entries) => return collect_directory_entries(entries),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                last_error = Some(error);
                transient_pause();
            }
            Err(error) => return Err(error),
        }
    }

    match fs::read_dir(root) {
        Ok(entries) => collect_directory_entries(entries),
        Err(error) => Err(last_error.unwrap_or(error)),
    }
}

fn collect_directory_entries(entries: fs::ReadDir) -> io::Result<Vec<fs::DirEntry>> {
    let mut collected = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => collected.push(entry),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                transient_pause();
            }
            Err(error) => return Err(error),
        }
    }
    Ok(collected)
}

fn transient_pause() {
    for _ in 0..256 {
        core::hint::spin_loop();
    }
}

fn read_app_about(app_root: &Path) -> Option<AppInfo> {
    let content = read_to_string(app_root.join("about.toml")).ok()?;

    let name = parse_string_field(&content, "name")?;
    let bundle_id = parse_string_field(&content, "bundle_id")
        .or_else(|| parse_string_field(&content, "bundle-id"))?;
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

fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    let mut last_error = None;
    for _ in 0..8 {
        match fs::read_to_string(path) {
            Ok(content) => return Ok(content),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                last_error = Some(error);
                transient_pause();
            }
            Err(error) => return Err(error),
        }
    }

    fs::read_to_string(path).or_else(|error| Err(last_error.unwrap_or(error)))
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
                other => other,
            });
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' => {
                in_string = false;
                values.push(current.clone());
            }
            other => current.push(other),
        }
    }

    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn formats_system_bar_clock_like_linux_backend() {
        let clock = clock_from_unix_seconds(1_704_067_140).unwrap_or_default();
        assert_eq!(clock.date, "12/31 Sun");
        assert_eq!(clock.time, "23:59");
    }

    #[test]
    fn encodes_capability_service_app_launch_request() {
        let executable = "/applications/test.app/entry.elf";
        let request = encode_spawn_app_request(executable, u64::MAX, true);
        assert!(request.is_ok());
        let request = request.unwrap_or_default();
        assert_eq!(request.len(), SPAWN_APP_HEADER_LEN + executable.len() + 1);
        assert_eq!(&request[0..4], &SPAWN_APP_OPCODE.to_le_bytes());
        assert_eq!(&request[4..8], &[0; 4]);
        assert_eq!(&request[8..16], &u64::MAX.to_le_bytes());
        assert_eq!(request[16], 1);
        assert_eq!(&request[17..24], &[0; 7]);
        assert_eq!(&request[24..], b"/applications/test.app/entry.elf\0");
    }

    #[test]
    fn app_launch_request_carries_session_environment() {
        let environment = vec![
            String::from("__MNU_EXEC_ENV=HOME=/home/alice"),
            String::from("__MNU_EXEC_ENV=USER=alice"),
        ];
        let request = encode_spawn_app_request_with_environment(
            "/applications/test.app/entry.elf",
            7,
            true,
            &environment,
        )
        .unwrap_or_default();
        assert_eq!(
            &request[SPAWN_APP_HEADER_LEN..],
            b"/applications/test.app/entry.elf\0__MNU_EXEC_ENV=HOME=/home/alice\0__MNU_EXEC_ENV=USER=alice\0"
        );
    }

    fn temporary_app_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "binder-app-discovery-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn discovers_valid_app_bundles_in_stable_order() {
        let root = temporary_app_root();
        let alpha = root.join("Alpha.app");
        let zeta = root.join("Zeta.app");
        let broken = root.join("Broken.app");
        assert!(fs::create_dir_all(&alpha).is_ok());
        assert!(fs::create_dir_all(&zeta).is_ok());
        assert!(fs::create_dir_all(&broken).is_ok());
        assert!(
            fs::write(
                alpha.join("about.toml"),
                "name = \"Alpha\"\nbundle_id = \"org.test.alpha\"\nentry = \"entry.elf\"\nicon = \"icon.png\"\n",
            )
            .is_ok()
        );
        assert!(
            fs::write(
                zeta.join("about.toml"),
                "name = \"Zeta\"\nbundle_id = \"org.test.zeta\"\nentry = \"internal:test\"\n",
            )
            .is_ok()
        );
        assert!(
            fs::write(
                broken.join("about.toml"),
                "name = \"Broken\"\nbundle_id = \"org.test.broken\"\n",
            )
            .is_ok()
        );

        let apps = read_apps_from(&root);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Alpha");
        assert_eq!(apps[0].icon, Some(alpha.join("icon.png")));
        assert_eq!(apps[1].name, "Zeta");
        assert_eq!(apps[1].entry, apps::TEST_ENTRY);

        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn missing_applications_root_is_an_empty_catalog() {
        let root = temporary_app_root();
        assert!(read_apps_from(&root).is_empty());
    }

    #[test]
    fn replaces_cached_application_catalog_after_installation() {
        let root = temporary_app_root();
        let installed = root.join("Installed.app");
        assert!(fs::create_dir_all(&installed).is_ok());
        assert!(
            fs::write(
                installed.join("about.toml"),
                "name = \"Installed\"\nbundle_id = \"org.test.installed\"\nentry = \"entry.elf\"\n",
            )
            .is_ok()
        );

        let mut platform = MochiOsPlatform::new();
        platform.apps.clear();
        let discovered = read_apps_from(&root);
        assert!(platform.replace_apps_if_changed(discovered.clone()));
        assert_eq!(platform.apps, discovered);
        assert!(!platform.replace_apps_if_changed(discovered));

        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn process_snapshot_excludes_terminated_processes() {
        let mut records = vec![0u8; PROCESS_RECORD_SIZE * 3];
        records[0..8].copy_from_slice(&7u64.to_ne_bytes());
        records[16..24].copy_from_slice(&1u64.to_ne_bytes());
        records[PROCESS_RECORD_SIZE..PROCESS_RECORD_SIZE + 8].copy_from_slice(&8u64.to_ne_bytes());
        records[PROCESS_RECORD_SIZE + 16..PROCESS_RECORD_SIZE + 24]
            .copy_from_slice(&PROCESS_STATE_TERMINATED.to_ne_bytes());
        records[PROCESS_RECORD_SIZE * 2..PROCESS_RECORD_SIZE * 2 + 8]
            .copy_from_slice(&9u64.to_ne_bytes());
        records[PROCESS_RECORD_SIZE * 2 + 16..PROCESS_RECORD_SIZE * 2 + 24]
            .copy_from_slice(&3u64.to_ne_bytes());

        let live = decode_live_process_ids(&records, 3);

        assert_eq!(live, HashSet::from([ProcessId(7), ProcessId(9)]));
    }

    #[test]
    fn exited_kernel_process_is_removed_from_running_apps() {
        let mut platform = MochiOsPlatform::new();
        platform.children.clear();
        platform.children.insert(
            ProcessId(7),
            ManagedApp {
                bundle_id: String::from("org.test.terminal"),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: true,
                linux_instance: None,
            },
        );
        platform.children.insert(
            ProcessId(0x4000_0000),
            ManagedApp {
                bundle_id: String::from("org.test.internal"),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: false,
                linux_instance: None,
            },
        );

        assert!(platform.remove_exited_kernel_processes(&HashSet::new()));
        assert!(platform.process_for_bundle("org.test.terminal").is_none());
        assert!(platform.process_for_bundle("org.test.internal").is_some());
        assert_eq!(platform.exited_processes, vec![ProcessId(7)]);
    }

    #[test]
    fn exited_linux_instance_is_removed_without_affecting_native_apps() {
        let mut platform = MochiOsPlatform::new();
        platform.children.clear();
        platform.children.insert(
            ProcessId(0x4000_0000),
            ManagedApp {
                bundle_id: String::from("org.mochios.linux.xterm"),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: false,
                linux_instance: Some(11),
            },
        );
        platform.children.insert(
            ProcessId(0x4000_0001),
            ManagedApp {
                bundle_id: String::from("org.mochios.linux.xclock"),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: false,
                linux_instance: Some(12),
            },
        );
        platform.children.insert(
            ProcessId(7),
            ManagedApp {
                bundle_id: String::from("org.mochios.files"),
                child: None,
                windows: HashSet::new(),
                track_kernel_lifecycle: true,
                linux_instance: None,
            },
        );

        assert!(platform.remove_exited_linux_processes(&HashSet::from([12])));
        assert!(
            platform
                .process_for_bundle("org.mochios.linux.xterm")
                .is_none()
        );
        assert!(
            platform
                .process_for_bundle("org.mochios.linux.xclock")
                .is_some()
        );
        assert!(platform.process_for_bundle("org.mochios.files").is_some());
        assert_eq!(platform.exited_processes, vec![ProcessId(0x4000_0000)]);
    }
}
