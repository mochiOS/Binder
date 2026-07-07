use std::collections::{HashMap, HashSet};

use std::process::{Child, Command, Stdio};

use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    ApplicationId, ClockState, DesktopPlatform, PlatformError, ProcessId, SystemAction,
    SystemBarState,
};

pub struct LinuxPlatform {
    system_bar: SystemBarState,

    children: HashMap<ProcessId, Child>,
    exited_processes: Vec<ProcessId>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            system_bar: read_system_bar_state().unwrap_or_default(),

            children: HashMap::new(),
            exited_processes: Vec::new(),
        }
    }

    fn reap_exited_children(&mut self) -> Result<(), PlatformError> {
        let mut exited = Vec::new();

        for (process_id, child) in &mut self.children {
            match child.try_wait() {
                Ok(Some(_status)) => {
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

            self.exited_processes.push(process_id);
        }

        Ok(())
    }

    fn terminate_child(&mut self, process_id: ProcessId) -> Result<(), PlatformError> {
        let Some(mut child) = self.children.remove(&process_id) else {
            return Ok(());
        };

        match child.try_wait() {
            Ok(Some(_status)) => {
                return Ok(());
            }

            Ok(None) => {}

            Err(_) => {
                return Err(PlatformError::ProcessTerminationFailed);
            }
        }

        child
            .kill()
            .map_err(|_| PlatformError::ProcessTerminationFailed)?;

        child
            .wait()
            .map_err(|_| PlatformError::ProcessTerminationFailed)?;

        Ok(())
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

    fn launch_application(
        &mut self,
        application: ApplicationId,
    ) -> Result<ProcessId, PlatformError> {
        let executable = std::env::current_exe().map_err(|_| PlatformError::ProcessLaunchFailed)?;

        let role = match application {
            ApplicationId::About => "--role=about",
        };

        let child = Command::new(executable)
            .arg(role)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| PlatformError::ProcessLaunchFailed)?;

        let process_id = ProcessId(child.id());

        self.children.insert(process_id, child);

        Ok(process_id)
    }

    fn synchronize_applications(
        &mut self,
        active_processes: &[ProcessId],
    ) -> Result<(), PlatformError> {
        let active: HashSet<ProcessId> = active_processes.iter().copied().collect();

        let stale_processes: Vec<ProcessId> = self
            .children
            .keys()
            .copied()
            .filter(|process_id| !active.contains(process_id))
            .collect();

        for process_id in stale_processes {
            self.terminate_child(process_id)?;
        }

        Ok(())
    }

    fn take_exited_processes(&mut self) -> Vec<ProcessId> {
        std::mem::take(&mut self.exited_processes)
    }

    fn refresh(&mut self) -> Result<bool, PlatformError> {
        self.reap_exited_children()?;

        let next = read_system_bar_state()?;

        let changed = next != self.system_bar;

        if changed {
            self.system_bar = next;
        }

        Ok(changed)
    }
}

impl Drop for LinuxPlatform {
    fn drop(&mut self) {
        for (_, mut child) in self.children.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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
