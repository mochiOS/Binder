use super::{DesktopPlatform, PlatformError, ProcessId, SystemAction, SystemBarState};

#[allow(unused)]
pub struct MochiOsPlatform {
    system_bar: SystemBarState,
}

impl MochiOsPlatform {
    #[allow(unused)]
    pub fn new() -> Self {
        Self {
            system_bar: SystemBarState::default(),
        }
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

    fn launch_internal_window(&mut self, _entry: &str) -> Result<ProcessId, PlatformError> {
        Err(PlatformError::UnsupportedOperation)
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
        Ok(false)
    }
}

#[allow(unused)]
pub(super) fn run_internal_process(_entry: &str) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedOperation)
}
