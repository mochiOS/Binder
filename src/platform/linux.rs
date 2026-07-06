use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

use super::{
    ClockState,
    DesktopPlatform,
    PlatformError,
    SystemBarState,
};

pub struct LinuxPlatform {
    system_bar: SystemBarState,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            system_bar:
            read_system_bar_state()
                .unwrap_or_default(),
        }
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPlatform for LinuxPlatform {
    fn system_bar_state(
        &self,
    ) -> Result<SystemBarState, PlatformError> {
        Ok(self.system_bar.clone())
    }

    fn open_system_menu(
        &self,
    ) -> Result<(), PlatformError> {
        // TODO: Binder内部のメニュー状態を開く処理へ接続する。
        Ok(())
    }

    fn refresh(
        &mut self,
    ) -> Result<bool, PlatformError> {
        let next =
            read_system_bar_state()?;

        let changed =
            next != self.system_bar;

        if changed {
            self.system_bar = next;
        }

        Ok(changed)
    }
}

fn read_system_bar_state(
) -> Result<SystemBarState, PlatformError> {
    Ok(SystemBarState {
        clock: read_clock()?,
        ..SystemBarState::default()
    })
}

fn read_clock(
) -> Result<ClockState, PlatformError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            PlatformError::InvalidResponse
        })?;

    let timestamp: libc::time_t =
        duration
            .as_secs()
            .try_into()
            .map_err(|_| {
                PlatformError::InvalidResponse
            })?;

    let mut local_time =
        std::mem::MaybeUninit::<libc::tm>
        ::uninit();

    let result = unsafe {
        libc::localtime_r(
            &timestamp,
            local_time.as_mut_ptr(),
        )
    };

    if result.is_null() {
        return Err(
            PlatformError::TransportFailure,
        );
    }

    let local_time = unsafe {
        local_time.assume_init()
    };

    let weekdays = [
        "Sun",
        "Mon",
        "Tue",
        "Wed",
        "Thu",
        "Fri",
        "Sat",
    ];

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

        time: format!(
            "{:02}:{:02}",
            local_time.tm_hour,
            local_time.tm_min,
        ),
    })
}