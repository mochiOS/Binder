use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::apps;

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

        let process_id = ProcessId(child.id());
        self.children.insert(
            process_id,
            ManagedApp {
                bundle_id: app.bundle_id.clone(),
                child: Some(child),
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
    let Ok(entries) = fs::read_dir(applications_root()) else {
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
        if let Some(app) = read_app_about(&path) {
            apps.push(app);
        }
    }

    apps.sort_by(|left, right| left.name.cmp(&right.name));
    apps
}

fn read_app_about(app_root: &Path) -> Option<AppInfo> {
    let content = fs::read_to_string(app_root.join("about.toml")).ok()?;

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
