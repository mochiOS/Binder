#[cfg(target_os = "linux")]
mod linux;

mod mochios;

use std::cell::RefCell;
use std::rc::Rc;

pub use mochios::MochiOsPlatform;

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
}

pub trait DesktopPlatform {
    fn system_bar_state(&self) -> Result<SystemBarState, PlatformError>;

    fn open_system_settings(
        &self,
    ) -> Result<(), PlatformError>;

    fn perform_system_action(
        &self,
        action: SystemAction,
    ) -> Result<(), PlatformError>;

    fn refresh(&mut self) -> Result<bool, PlatformError>;
}

pub fn current() -> Rc<RefCell<dyn DesktopPlatform>> {
    #[cfg(target_os = "linux")]
    {
        Rc::new(RefCell::new(
            linux::LinuxPlatform::new(),
        ))
    }

    #[cfg(not(target_os = "linux"))]
    {
        Rc::new(RefCell::new(
            mochios::MochiOsPlatform::new(),
        ))
    }
}