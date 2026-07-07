#[cfg(target_os = "linux")]
mod linux;

mod mochios;

use std::cell::RefCell;
use std::rc::Rc;

pub use crate::ipc::{ApplicationId, RemoteWindowId};

#[cfg(not(target_os = "linux"))]
pub use mochios::MochiOsPlatform;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateWindowRequest {
    pub process_id: ProcessId,
    pub application: ApplicationId,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockState {
    pub date: String,
    pub time: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationState {
    pub unread_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
}

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

    fn launch_application(
        &mut self,
        _application: ApplicationId,
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

pub fn run_application_process(application: ApplicationId) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        linux::run_application_process(application)
    }

    #[cfg(not(target_os = "linux"))]
    {
        mochios::run_application_process(application)
    }
}
