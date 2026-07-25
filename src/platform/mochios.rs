use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;

#[cfg(not(target_os = "mochios"))]
use std::process::{Command, Stdio};

use crate::apps;

#[cfg(target_os = "mochios")]
mod decoration;

use super::{
    AppInfo, CloseWindowRequest, CreateWindowRequest, DesktopPlatform, PlatformError, ProcessId,
    RemoteWindowId, SystemAction, SystemBarState,
};

#[derive(Debug)]
struct ManagedApp {
    bundle_id: String,
    child: Option<Child>,
    windows: HashSet<RemoteWindowId>,
}

#[allow(unused)]
pub struct MochiOsPlatform {
    system_bar: SystemBarState,
    apps: Vec<AppInfo>,
    children: HashMap<ProcessId, ManagedApp>,
    create_window_requests: Vec<CreateWindowRequest>,
    close_window_requests: Vec<CloseWindowRequest>,
    exited_processes: Vec<ProcessId>,
    next_internal_pid: u32,
    #[cfg(target_os = "mochios")]
    decoration_manager: Option<decoration::DecorationManager>,
}

impl MochiOsPlatform {
    #[allow(unused)]
    pub fn new() -> Self {
        Self {
            system_bar: SystemBarState::default(),
            apps: read_apps(),
            children: HashMap::new(),
            create_window_requests: Vec::new(),
            close_window_requests: Vec::new(),
            exited_processes: Vec::new(),
            next_internal_pid: 0x4000_0000,
            #[cfg(target_os = "mochios")]
            decoration_manager: decoration::DecorationManager::connect()
                .map_err(|error| eprintln!("Binder decoration manager unavailable: {error}"))
                .ok(),
        }
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

        let changed = !exited.is_empty();
        for process_id in exited {
            self.children.remove(&process_id);
            self.exited_processes.push(process_id);
        }

        Ok(changed)
    }
}

#[cfg(target_os = "mochios")]
fn spawn_application(app: &AppInfo) -> Result<(ProcessId, Option<Child>), PlatformError> {
    const CAPABILITY_SERVICE_NAME: &str = "capability.service";
    use mochi_user_syscall as syscall;

    let executable = app.entry_path();
    let executable = executable
        .to_str()
        .ok_or(PlatformError::ProcessLaunchFailed)?;
    let request = encode_spawn_app_request(
        executable,
        shell_endpoint_from_environment(),
        prompt_is_interactive(),
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
        return Err(PlatformError::ProcessLaunchFailed);
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

fn encode_spawn_app_request(
    executable: &str,
    shell_endpoint: u64,
    interactive: bool,
) -> Result<Vec<u8>, PlatformError> {
    if executable.is_empty() || !executable.starts_with('/') || executable.as_bytes().contains(&0) {
        return Err(PlatformError::ProcessLaunchFailed);
    }

    let mut request = vec![0u8; SPAWN_APP_HEADER_LEN + executable.len() + 1];
    request[0..4].copy_from_slice(&SPAWN_APP_OPCODE.to_le_bytes());
    request[8..16].copy_from_slice(&shell_endpoint.to_le_bytes());
    request[16] = u8::from(interactive);
    request[SPAWN_APP_HEADER_LEN..SPAWN_APP_HEADER_LEN + executable.len()]
        .copy_from_slice(executable.as_bytes());
    Ok(request)
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

    fn open_system_settings(&self) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn perform_system_action(&self, _action: SystemAction) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
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
        if let Some(manager) = self.decoration_manager.as_mut()
            && let Err(error) = manager.poll()
        {
            eprintln!("Binder decoration manager failed: {error}");
            self.decoration_manager = None;
        }

        self.reap_exited_children()
    }

    fn get_apps(&self) -> Vec<AppInfo> {
        self.apps.clone()
    }

    fn launch_app(&mut self, app: &AppInfo) -> Result<ProcessId, PlatformError> {
        match app.entry.as_str() {
            apps::ABOUT_ENTRY | apps::TEST_ENTRY => {
                self.launch_internal_renderer(&app.entry, app.bundle_id.clone())
            }
            _ => self.spawn_entry(app),
        }
    }

    fn running_app_bundle_ids(&self) -> Vec<String> {
        self.children
            .values()
            .map(|child| child.bundle_id.clone())
            .collect()
    }
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
        Box::new(crate::ui::test::view())
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
    read_apps_from(&applications_root())
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
        if let Some(app) = read_app_about(&path) {
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
}
