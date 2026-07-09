#[cfg(target_os = "linux")]
mod linux;

mod mochios;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub use crate::ipc::RemoteWindowId;

#[cfg(not(target_os = "linux"))]
pub use mochios::MochiOsPlatform;

use crate::window::WindowContent;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateWindowRequest {
    pub process_id: ProcessId,
    pub content: WindowContent,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseWindowRequest {
    pub process_id: ProcessId,
    pub window: RemoteWindowId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowResizedNotification {
    pub process_id: ProcessId,
    pub window: RemoteWindowId,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowFocusChangedNotification {
    pub process_id: ProcessId,
    pub window: RemoteWindowId,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockState {
    pub date: String,
    pub time: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationState {
    pub unread_count: u32,
}

#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
}

#[allow(unused)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkState {
    Unavailable,
    Disconnected,
    Connecting,

    Connected {
        network_name: Option<String>,
        signal_strength: Option<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    pub root: PathBuf,
    pub name: String,
    pub bundle_id: String,
    pub version: String,
    pub developer: String,
    pub entry: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub resources: Vec<PathBuf>,
}

impl AppInfo {
    pub fn entry_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.entry);

        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::Unavailable
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeState {
    pub available: bool,
    pub muted: bool,
    pub level: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatteryState {
    pub available: bool,
    pub charging: bool,
    pub percentage: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemBarState {
    pub clock: ClockState,
    pub notifications: NotificationState,
    pub network: NetworkState,
    pub volume: VolumeState,
    pub battery: BatteryState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    ServiceUnavailable,
    InvalidResponse,
    PermissionDenied,
    TransportFailure,
    UnsupportedOperation,
    ProcessLaunchFailed,
    ProcessTerminationFailed,
}

pub trait DesktopPlatform {
    fn system_bar_state(&self) -> Result<SystemBarState, PlatformError>;

    fn open_system_settings(&self) -> Result<(), PlatformError>;

    fn perform_system_action(&self, action: SystemAction) -> Result<(), PlatformError>;

    fn launch_internal_window(
        &mut self,
        _content: WindowContent,
    ) -> Result<ProcessId, PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn register_window(
        &mut self,
        _process_id: ProcessId,
        _window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn request_window_close(
        &mut self,
        _process_id: ProcessId,
        _window: RemoteWindowId,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn notify_window_resized(
        &mut self,
        _notification: WindowResizedNotification,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn notify_window_focus_changed(
        &mut self,
        _notification: WindowFocusChangedNotification,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn synchronize_applications(
        &mut self,
        _active_processes: &[ProcessId],
    ) -> Result<(), PlatformError> {
        Ok(())
    }

    fn take_create_window_requests(&mut self) -> Vec<CreateWindowRequest> {
        Vec::new()
    }

    fn take_close_window_requests(&mut self) -> Vec<CloseWindowRequest> {
        Vec::new()
    }

    fn take_exited_processes(&mut self) -> Vec<ProcessId> {
        Vec::new()
    }

    fn refresh(&mut self) -> Result<bool, PlatformError>;

    fn get_apps(&self) -> Vec<AppInfo> {
        Vec::new()
    }

    fn launch_app(&mut self, _app: &AppInfo) -> Result<ProcessId, PlatformError> {
        Err(PlatformError::UnsupportedOperation)
    }

    fn running_app_bundle_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

pub fn current() -> Rc<RefCell<dyn DesktopPlatform>> {
    #[cfg(target_os = "linux")]
    {
        Rc::new(RefCell::new(linux::LinuxPlatform::new()))
    }

    #[cfg(not(target_os = "linux"))]
    {
        Rc::new(RefCell::new(mochios::MochiOsPlatform::new()))
    }
}

pub fn run_internal_process(content: WindowContent) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        linux::run_internal_process(content)
    }

    #[cfg(not(target_os = "linux"))]
    {
        mochios::run_internal_process(content)
    }
}
