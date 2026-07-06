use super::{DesktopPlatform, PlatformError, SystemBarState};

pub struct MochiOsPlatform {
    system_bar: SystemBarState,
}

impl MochiOsPlatform {
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

    fn open_system_menu(&self) -> Result<(), PlatformError> {
        // TODO: Binder内部のメニュー状態を開く処理へ接続する。
        Ok(())
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
