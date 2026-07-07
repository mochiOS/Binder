use std::collections::{HashMap, HashSet};

use std::fs;

use std::io::{self, Read};

use std::os::fd::AsRawFd;

use std::os::unix::fs::DirBuilderExt;

use std::os::unix::net::{UnixListener, UnixStream};

use std::path::PathBuf;

use std::process::{Child, Command, Stdio};

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ipc::{BINDER_SOCKET_ENV, CLIENT_PACKET_SIZE, ClientRequest, decode_client_request};

use super::{
    ApplicationId, ClockState, CreateWindowRequest, DesktopPlatform, PlatformError, ProcessId,
    SystemAction, SystemBarState,
};

const APPLICATION_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

struct ManagedChild {
    application: ApplicationId,
    child: Child,

    registered: bool,
    launched_at: Instant,
}

struct PendingClient {
    stream: UnixStream,
    process_id: ProcessId,

    packet: [u8; CLIENT_PACKET_SIZE],

    received: usize,
}

pub struct LinuxPlatform {
    system_bar: SystemBarState,

    listener: UnixListener,
    socket_directory: PathBuf,
    socket_path: PathBuf,

    children: HashMap<ProcessId, ManagedChild>,

    pending_clients: Vec<PendingClient>,

    create_window_requests: Vec<CreateWindowRequest>,

    exited_processes: Vec<ProcessId>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self::try_new().unwrap_or_else(|error| {
            panic!("failed to initialize Binder IPC: {error:?}",);
        })
    }

    fn try_new() -> Result<Self, PlatformError> {
        let (listener, socket_directory, socket_path) = create_listener()?;

        Ok(Self {
            system_bar: read_system_bar_state().unwrap_or_default(),

            listener,
            socket_directory,
            socket_path,

            children: HashMap::new(),

            pending_clients: Vec::new(),

            create_window_requests: Vec::new(),

            exited_processes: Vec::new(),
        })
    }

    fn reap_exited_children(&mut self) -> Result<(), PlatformError> {
        let mut exited = Vec::new();

        for (process_id, managed_child) in &mut self.children {
            match managed_child.child.try_wait() {
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

            self.pending_clients
                .retain(|client| client.process_id != process_id);

            self.exited_processes.push(process_id);
        }

        Ok(())
    }

    fn terminate_child(&mut self, process_id: ProcessId) -> Result<(), PlatformError> {
        let Some(mut managed_child) = self.children.remove(&process_id) else {
            return Ok(());
        };

        self.pending_clients
            .retain(|client| client.process_id != process_id);

        match managed_child.child.try_wait() {
            Ok(Some(_status)) => {
                return Ok(());
            }

            Ok(None) => {}

            Err(_) => {
                return Err(PlatformError::ProcessTerminationFailed);
            }
        }

        managed_child
            .child
            .kill()
            .map_err(|_| PlatformError::ProcessTerminationFailed)?;

        managed_child
            .child
            .wait()
            .map_err(|_| PlatformError::ProcessTerminationFailed)?;

        Ok(())
    }

    fn accept_clients(&mut self) -> Result<(), PlatformError> {
        loop {
            let accepted = self.listener.accept();

            let (stream, _address) = match accepted {
                Ok(connection) => connection,

                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }

                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }

                Err(_) => {
                    return Err(PlatformError::TransportFailure);
                }
            };

            let process_id = match peer_process_id(&stream) {
                Ok(process_id) => process_id,

                Err(_) => {
                    continue;
                }
            };

            if !self.children.contains_key(&process_id) {
                continue;
            }

            stream
                .set_nonblocking(true)
                .map_err(|_| PlatformError::TransportFailure)?;

            self.pending_clients.push(PendingClient {
                stream,
                process_id,

                packet: [0; CLIENT_PACKET_SIZE],

                received: 0,
            });
        }

        Ok(())
    }

    fn poll_pending_clients(&mut self) {
        let mut completed = Vec::new();

        let mut remove_indices = Vec::new();

        for (index, client) in self.pending_clients.iter_mut().enumerate() {
            loop {
                let remaining = &mut client.packet[client.received..];

                match client.stream.read(remaining) {
                    Ok(0) => {
                        remove_indices.push(index);

                        break;
                    }

                    Ok(read) => {
                        client.received += read;

                        if client.received == CLIENT_PACKET_SIZE {
                            match decode_client_request(&client.packet) {
                                Ok(request) => {
                                    completed.push((client.process_id, request));
                                }

                                Err(error) => {
                                    eprintln!(
                                        "invalid Binder IPC request from {:?}: {error:?}",
                                        client.process_id,
                                    );
                                }
                            }

                            remove_indices.push(index);

                            break;
                        }
                    }

                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        break;
                    }

                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        continue;
                    }

                    Err(_) => {
                        remove_indices.push(index);

                        break;
                    }
                }
            }
        }

        remove_indices.sort_unstable();
        remove_indices.dedup();

        for index in remove_indices.into_iter().rev() {
            self.pending_clients.swap_remove(index);
        }

        for (process_id, request) in completed {
            self.handle_client_request(process_id, request);
        }
    }

    fn handle_client_request(&mut self, process_id: ProcessId, request: ClientRequest) {
        match request {
            ClientRequest::CreateWindow { application } => {
                let Some(managed_child) = self.children.get_mut(&process_id) else {
                    return;
                };

                if managed_child.application != application {
                    eprintln!(
                        "Binder child {:?} requested an invalid application role",
                        process_id,
                    );

                    return;
                }

                if managed_child.registered {
                    return;
                }

                managed_child.registered = true;

                self.create_window_requests.push(CreateWindowRequest {
                    process_id,
                    application,
                });
            }
        }
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
        if let Some(process_id) = self
            .children
            .iter()
            .find_map(|(process_id, managed_child)| {
                (managed_child.application == application).then_some(*process_id)
            })
        {
            return Ok(process_id);
        }

        let executable = std::env::current_exe().map_err(|error| {
            eprintln!("failed to locate Binder executable: {error}",);

            PlatformError::ProcessLaunchFailed
        })?;

        let argument = match application {
            ApplicationId::About => "--role=about",
        };

        let child = Command::new(executable)
            .arg(argument)
            .env(BINDER_SOCKET_ENV, &self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                eprintln!("failed to launch Binder child process: {error}",);

                PlatformError::ProcessLaunchFailed
            })?;

        let process_id = ProcessId(child.id());

        self.children.insert(
            process_id,
            ManagedChild {
                application,
                child,

                registered: false,
                launched_at: Instant::now(),
            },
        );

        Ok(process_id)
    }

    fn synchronize_applications(
        &mut self,
        active_processes: &[ProcessId],
    ) -> Result<(), PlatformError> {
        let active: HashSet<ProcessId> = active_processes.iter().copied().collect();

        let stale_processes: Vec<ProcessId> = self
            .children
            .iter()
            .filter_map(|(process_id, managed_child)| {
                let window_closed = managed_child.registered && !active.contains(process_id);

                let registration_timed_out = !managed_child.registered
                    && managed_child.launched_at.elapsed() >= APPLICATION_REGISTRATION_TIMEOUT;

                (window_closed || registration_timed_out).then_some(*process_id)
            })
            .collect();

        for process_id in stale_processes {
            self.terminate_child(process_id)?;
        }

        Ok(())
    }

    fn take_create_window_requests(&mut self) -> Vec<CreateWindowRequest> {
        std::mem::take(&mut self.create_window_requests)
    }

    fn take_exited_processes(&mut self) -> Vec<ProcessId> {
        std::mem::take(&mut self.exited_processes)
    }

    fn refresh(&mut self) -> Result<bool, PlatformError> {
        self.reap_exited_children()?;

        self.accept_clients()?;
        self.poll_pending_clients();

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
        for (_process_id, mut managed_child) in self.children.drain() {
            let _ = managed_child.child.kill();

            let _ = managed_child.child.wait();
        }

        let _ = fs::remove_file(&self.socket_path);

        let _ = fs::remove_dir(&self.socket_directory);
    }
}

fn create_listener() -> Result<(UnixListener, PathBuf, PathBuf), PlatformError> {
    let effective_user_id = unsafe { libc::geteuid() };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PlatformError::TransportFailure)?
        .as_nanos();

    let socket_directory = std::env::temp_dir().join(format!(
        "mochios-binder-{}-{}-{}",
        effective_user_id,
        std::process::id(),
        nonce,
    ));

    let mut directory_builder = fs::DirBuilder::new();

    directory_builder.mode(0o700);

    directory_builder
        .create(&socket_directory)
        .map_err(|_| PlatformError::TransportFailure)?;

    let socket_path = socket_directory.join("control.sock");

    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,

        Err(_) => {
            let _ = fs::remove_dir(&socket_directory);

            return Err(PlatformError::TransportFailure);
        }
    };

    if listener.set_nonblocking(true).is_err() {
        let _ = fs::remove_file(&socket_path);

        let _ = fs::remove_dir(&socket_directory);

        return Err(PlatformError::TransportFailure);
    }

    Ok((listener, socket_directory, socket_path))
}

fn peer_process_id(stream: &UnixStream) -> Result<ProcessId, PlatformError> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };

    let mut credentials_length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut libc::ucred as *mut libc::c_void,
            &mut credentials_length,
        )
    };

    if result != 0 {
        return Err(PlatformError::TransportFailure);
    }

    let effective_user_id = unsafe { libc::geteuid() };

    if credentials.uid != effective_user_id {
        return Err(PlatformError::PermissionDenied);
    }

    let process_id: u32 = credentials
        .pid
        .try_into()
        .map_err(|_| PlatformError::InvalidResponse)?;

    Ok(ProcessId(process_id))
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
