use crate::platform::ApplicationId;

pub(crate) const BINDER_SOCKET_ENV: &str = "BINDER_SOCKET";

pub(crate) const CLIENT_PACKET_SIZE: usize = 8;

const PROTOCOL_MAGIC: &[u8; 4] = b"BNDR";
const PROTOCOL_VERSION: u8 = 1;

const OPCODE_CREATE_WINDOW: u8 = 1;

const APPLICATION_ABOUT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientRequest {
    CreateWindow { application: ApplicationId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    InvalidMagic,
    UnsupportedVersion,
    UnknownOpcode,
    UnknownApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientIpcError {
    MissingSocket,
    ConnectionFailed,
    WriteFailed,
    UnsupportedPlatform,
}

pub(crate) fn encode_create_window(application: ApplicationId) -> [u8; CLIENT_PACKET_SIZE] {
    let application_code = match application {
        ApplicationId::About => APPLICATION_ABOUT,
    };

    [
        PROTOCOL_MAGIC[0],
        PROTOCOL_MAGIC[1],
        PROTOCOL_MAGIC[2],
        PROTOCOL_MAGIC[3],
        PROTOCOL_VERSION,
        OPCODE_CREATE_WINDOW,
        application_code,
        0,
    ]
}

pub(crate) fn decode_client_request(
    packet: &[u8; CLIENT_PACKET_SIZE],
) -> Result<ClientRequest, ProtocolError> {
    if &packet[0..4] != PROTOCOL_MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }

    if packet[4] != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }

    let application = match packet[6] {
        APPLICATION_ABOUT => ApplicationId::About,

        _ => {
            return Err(ProtocolError::UnknownApplication);
        }
    };

    match packet[5] {
        OPCODE_CREATE_WINDOW => Ok(ClientRequest::CreateWindow { application }),

        _ => Err(ProtocolError::UnknownOpcode),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn send_create_window(application: ApplicationId) -> Result<(), ClientIpcError> {
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let socket_path = std::env::var_os(BINDER_SOCKET_ENV).ok_or(ClientIpcError::MissingSocket)?;

    let mut stream =
        UnixStream::connect(socket_path).map_err(|_| ClientIpcError::ConnectionFailed)?;

    let packet = encode_create_window(application);

    stream
        .write_all(&packet)
        .map_err(|_| ClientIpcError::WriteFailed)?;

    stream.flush().map_err(|_| ClientIpcError::WriteFailed)?;

    stream
        .shutdown(Shutdown::Write)
        .map_err(|_| ClientIpcError::WriteFailed)?;

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn send_create_window(_application: ApplicationId) -> Result<(), ClientIpcError> {
    Err(ClientIpcError::UnsupportedPlatform)
}
