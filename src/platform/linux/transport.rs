use std::collections::{HashMap, HashSet, VecDeque};

use std::fs;

use std::io::{self, Read, Write};

use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;

use std::os::unix::net::{UnixListener, UnixStream};

use std::path::{Path, PathBuf};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::ipc::{
    ApplicationId, ClientRequest, ServerEvent, encode_client_request, encode_server_event,
    try_decode_client_request, try_decode_server_event,
};

use crate::platform::{PlatformError, ProcessId};

pub(super) const BINDER_SOCKET_ENV: &str = "BINDER_SOCKET";

pub(super) enum TransportEvent {
    Request {
        process_id: ProcessId,
        request: ClientRequest,
    },

    Disconnected {
        process_id: ProcessId,
    },
}

struct PendingWrite {
    bytes: Vec<u8>,
    offset: usize,
}

struct ClientConnection {
    stream: UnixStream,
    read_buffer: Vec<u8>,
    writes: VecDeque<PendingWrite>,
}

impl ClientConnection {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            read_buffer: Vec::new(),
            writes: VecDeque::new(),
        }
    }

    fn queue(&mut self, bytes: Vec<u8>) {
        self.writes.push_back(PendingWrite { bytes, offset: 0 });
    }
}

pub(super) struct LinuxIpcServer {
    listener: UnixListener,
    socket_directory: PathBuf,
    socket_path: PathBuf,

    clients: HashMap<ProcessId, ClientConnection>,
}

impl LinuxIpcServer {
    pub(super) fn new() -> Result<Self, PlatformError> {
        let (listener, socket_directory, socket_path) = create_listener()?;

        Ok(Self {
            listener,
            socket_directory,
            socket_path,
            clients: HashMap::new(),
        })
    }

    pub(super) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub(super) fn send_event(
        &mut self,
        process_id: ProcessId,
        event: ServerEvent,
    ) -> Result<(), PlatformError> {
        let bytes = encode_server_event(&event).map_err(|_| PlatformError::InvalidResponse)?;

        let connection = self
            .clients
            .get_mut(&process_id)
            .ok_or(PlatformError::ServiceUnavailable)?;

        connection.queue(bytes);

        Ok(())
    }

    pub(super) fn remove_client(&mut self, process_id: ProcessId) {
        self.clients.remove(&process_id);
    }

    pub(super) fn poll(
        &mut self,
        allowed_processes: &HashSet<ProcessId>,
    ) -> Result<Vec<TransportEvent>, PlatformError> {
        self.accept_clients(allowed_processes)?;

        let process_ids: Vec<ProcessId> = self.clients.keys().copied().collect();

        let mut events = Vec::new();

        let mut disconnected = Vec::new();

        for process_id in process_ids {
            let result = {
                let Some(connection) = self.clients.get_mut(&process_id) else {
                    continue;
                };

                poll_connection(connection)
            };

            match result {
                Ok(result) => {
                    for request in result.requests {
                        events.push(TransportEvent::Request {
                            process_id,
                            request,
                        });
                    }

                    if result.disconnected {
                        disconnected.push(process_id);
                    }
                }

                Err(_) => {
                    disconnected.push(process_id);
                }
            }
        }

        disconnected.sort_by_key(|process_id| process_id.0);

        disconnected.dedup();

        for process_id in disconnected {
            self.clients.remove(&process_id);

            events.push(TransportEvent::Disconnected { process_id });
        }

        Ok(events)
    }

    fn accept_clients(
        &mut self,
        allowed_processes: &HashSet<ProcessId>,
    ) -> Result<(), PlatformError> {
        loop {
            let accepted = self.listener.accept();

            let (stream, _) = match accepted {
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

            if !allowed_processes.contains(&process_id) {
                continue;
            }

            if self.clients.contains_key(&process_id) {
                continue;
            }

            stream
                .set_nonblocking(true)
                .map_err(|_| PlatformError::TransportFailure)?;

            self.clients
                .insert(process_id, ClientConnection::new(stream));
        }

        Ok(())
    }
}

impl Drop for LinuxIpcServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);

        let _ = fs::remove_dir(&self.socket_directory);
    }
}

struct ConnectionPoll {
    requests: Vec<ClientRequest>,
    disconnected: bool,
}

fn poll_connection(connection: &mut ClientConnection) -> Result<ConnectionPoll, PlatformError> {
    let mut disconnected = false;

    loop {
        let mut buffer = [0_u8; 1024];

        match connection.stream.read(&mut buffer) {
            Ok(0) => {
                disconnected = true;
                break;
            }

            Ok(read) => {
                connection.read_buffer.extend_from_slice(&buffer[..read]);
            }

            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                break;
            }

            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                continue;
            }

            Err(_) => {
                disconnected = true;
                break;
            }
        }
    }

    let mut requests = Vec::new();

    loop {
        match try_decode_client_request(&mut connection.read_buffer) {
            Ok(Some(request)) => {
                requests.push(request);
            }

            Ok(None) => break,

            Err(_) => {
                return Err(PlatformError::InvalidResponse);
            }
        }
    }

    if !disconnected {
        disconnected = !flush_writes(connection)?;
    }

    Ok(ConnectionPoll {
        requests,
        disconnected,
    })
}

fn flush_writes(connection: &mut ClientConnection) -> Result<bool, PlatformError> {
    loop {
        let completed = {
            let Some(write) = connection.writes.front_mut() else {
                return Ok(true);
            };

            match connection.stream.write(&write.bytes[write.offset..]) {
                Ok(0) => {
                    return Ok(false);
                }

                Ok(written) => {
                    write.offset += written;

                    write.offset == write.bytes.len()
                }

                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(true);
                }

                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }

                Err(_) => {
                    return Ok(false);
                }
            }
        };

        if completed {
            connection.writes.pop_front();
        }
    }
}

pub(super) fn run_application_process(application: ApplicationId) -> Result<(), PlatformError> {
    let socket_path =
        std::env::var_os(BINDER_SOCKET_ENV).ok_or(PlatformError::ServiceUnavailable)?;

    let mut stream =
        UnixStream::connect(socket_path).map_err(|_| PlatformError::TransportFailure)?;

    let request = create_window_request(application);

    let frame = encode_client_request(&request).map_err(|_| PlatformError::InvalidResponse)?;

    stream
        .write_all(&frame)
        .map_err(|_| PlatformError::TransportFailure)?;

    stream
        .flush()
        .map_err(|_| PlatformError::TransportFailure)?;

    let mut read_buffer = Vec::new();

    let mut created_window = None;

    loop {
        let mut buffer = [0_u8; 1024];

        let read = stream
            .read(&mut buffer)
            .map_err(|_| PlatformError::TransportFailure)?;

        if read == 0 {
            return Err(PlatformError::TransportFailure);
        }

        read_buffer.extend_from_slice(&buffer[..read]);

        loop {
            let event = try_decode_server_event(&mut read_buffer)
                .map_err(|_| PlatformError::InvalidResponse)?;

            let Some(event) = event else {
                break;
            };

            match event {
                ServerEvent::WindowCreated { window } => {
                    created_window = Some(window);
                }

                ServerEvent::CloseRequested { window } => {
                    if created_window != Some(window) {
                        continue;
                    }

                    let response = ClientRequest::CloseWindow { window };

                    let frame = encode_client_request(&response)
                        .map_err(|_| PlatformError::InvalidResponse)?;

                    stream
                        .write_all(&frame)
                        .map_err(|_| PlatformError::TransportFailure)?;

                    stream
                        .flush()
                        .map_err(|_| PlatformError::TransportFailure)?;

                    return Ok(());
                }
            }
        }
    }
}

fn create_window_request(application: ApplicationId) -> ClientRequest {
    match application {
        ApplicationId::About => ClientRequest::CreateWindow {
            application,
            title: String::from("About mochiOS"),
            width: 420,
            height: 300,
            resizable: true,
        },
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

    let mut builder = fs::DirBuilder::new();

    builder.mode(0o700);

    builder
        .create(&socket_directory)
        .map_err(|_| PlatformError::TransportFailure)?;

    let socket_path = socket_directory.join("control.sock");

    let listener = UnixListener::bind(&socket_path).map_err(|_| PlatformError::TransportFailure)?;

    listener
        .set_nonblocking(true)
        .map_err(|_| PlatformError::TransportFailure)?;

    Ok((listener, socket_directory, socket_path))
}

fn peer_process_id(stream: &UnixStream) -> Result<ProcessId, PlatformError> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };

    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut libc::ucred as *mut libc::c_void,
            &mut length,
        )
    };

    if result != 0 {
        return Err(PlatformError::TransportFailure);
    }

    let effective_user_id = unsafe { libc::geteuid() };

    if credentials.uid != effective_user_id {
        return Err(PlatformError::PermissionDenied);
    }

    let process_id = u32::try_from(credentials.pid).map_err(|_| PlatformError::InvalidResponse)?;

    Ok(ProcessId(process_id))
}
