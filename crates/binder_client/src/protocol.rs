const PROTOCOL_MAGIC: [u8; 4] = *b"BNDR";
const PROTOCOL_VERSION: u8 = 1;

const HEADER_SIZE: usize = 12;
const MAX_PAYLOAD_SIZE: usize = 4096;
const MAX_TITLE_SIZE: usize = 256;
const MAX_WINDOW_SIZE: u32 = 16_384;

const CLIENT_CREATE_WINDOW: u8 = 1;
const CLIENT_CLOSE_WINDOW: u8 = 2;

const SERVER_WINDOW_CREATED: u8 = 129;
const SERVER_CLOSE_REQUESTED: u8 = 130;
const SERVER_RESIZED: u8 = 131;
const SERVER_FOCUS_CHANGED: u8 = 132;

const APPLICATION_ABOUT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ApplicationId {
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RemoteWindowId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClientRequest {
    CreateWindow {
        application: ApplicationId,
        title: String,
        width: u32,
        height: u32,
        resizable: bool,
    },

    CloseWindow {
        window: RemoteWindowId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerEvent {
    WindowCreated {
        window: RemoteWindowId,
    },

    CloseRequested {
        window: RemoteWindowId,
    },

    Resized {
        window: RemoteWindowId,
        width: u32,
        height: u32,
    },

    FocusChanged {
        window: RemoteWindowId,
        focused: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    InvalidMagic,
    UnsupportedVersion,
    PayloadTooLarge,
    InvalidPayload,
    InvalidMessage,
    InvalidTitle,
    InvalidWindowSize,
}

pub(crate) fn encode_client_request(request: &ClientRequest) -> Result<Vec<u8>, ProtocolError> {
    match request {
        ClientRequest::CreateWindow {
            application,
            title,
            width,
            height,
            resizable,
        } => {
            validate_window_size(*width, *height)?;

            let title_bytes = title.as_bytes();

            if title_bytes.is_empty() || title_bytes.len() > MAX_TITLE_SIZE {
                return Err(ProtocolError::InvalidTitle);
            }

            let mut payload = Vec::with_capacity(12 + title_bytes.len());

            payload.push(encode_application(*application));
            payload.push(u8::from(*resizable));
            payload.extend_from_slice(&width.to_le_bytes());
            payload.extend_from_slice(&height.to_le_bytes());
            payload.extend_from_slice(&(title_bytes.len() as u16).to_le_bytes());
            payload.extend_from_slice(title_bytes);

            encode_frame(CLIENT_CREATE_WINDOW, &payload)
        }

        ClientRequest::CloseWindow { window } => {
            encode_frame(CLIENT_CLOSE_WINDOW, &window.0.to_le_bytes())
        }
    }
}

pub(crate) fn try_decode_server_event(
    buffer: &mut Vec<u8>,
) -> Result<Option<ServerEvent>, ProtocolError> {
    let Some((kind, payload)) = take_frame(buffer)? else {
        return Ok(None);
    };

    let event = match kind {
        SERVER_WINDOW_CREATED => {
            let mut reader = PayloadReader::new(&payload);

            let window = RemoteWindowId(reader.read_u64()?);

            reader.finish()?;

            ServerEvent::WindowCreated { window }
        }

        SERVER_CLOSE_REQUESTED => {
            let mut reader = PayloadReader::new(&payload);

            let window = RemoteWindowId(reader.read_u64()?);

            reader.finish()?;

            ServerEvent::CloseRequested { window }
        }

        SERVER_RESIZED => {
            let mut reader = PayloadReader::new(&payload);

            let window = RemoteWindowId(reader.read_u64()?);
            let width = reader.read_u32()?;
            let height = reader.read_u32()?;

            validate_window_size(width, height)?;

            reader.finish()?;

            ServerEvent::Resized {
                window,
                width,
                height,
            }
        }

        SERVER_FOCUS_CHANGED => {
            let mut reader = PayloadReader::new(&payload);

            let window = RemoteWindowId(reader.read_u64()?);

            let focused = match reader.read_u8()? {
                0 => false,
                1 => true,
                _ => {
                    return Err(ProtocolError::InvalidPayload);
                }
            };

            reader.finish()?;

            ServerEvent::FocusChanged { window, focused }
        }

        _ => {
            return Err(ProtocolError::InvalidMessage);
        }
    };

    Ok(Some(event))
}

fn encode_frame(kind: u8, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(ProtocolError::PayloadTooLarge);
    }

    let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());

    frame.extend_from_slice(&PROTOCOL_MAGIC);
    frame.push(PROTOCOL_VERSION);
    frame.push(kind);
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);

    Ok(frame)
}

fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<(u8, Vec<u8>)>, ProtocolError> {
    if buffer.len() < HEADER_SIZE {
        return Ok(None);
    }

    if buffer.get(0..4) != Some(PROTOCOL_MAGIC.as_slice()) {
        return Err(ProtocolError::InvalidMagic);
    }

    if buffer[4] != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }

    let kind = buffer[5];

    let payload_length =
        u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]) as usize;

    if payload_length > MAX_PAYLOAD_SIZE {
        return Err(ProtocolError::PayloadTooLarge);
    }

    let frame_length = HEADER_SIZE + payload_length;

    if buffer.len() < frame_length {
        return Ok(None);
    }

    let payload = buffer[HEADER_SIZE..frame_length].to_vec();

    buffer.drain(..frame_length);

    Ok(Some((kind, payload)))
}

fn encode_application(application: ApplicationId) -> u8 {
    match application {
        ApplicationId::About => APPLICATION_ABOUT,
    }
}

fn validate_window_size(width: u32, height: u32) -> Result<(), ProtocolError> {
    if width == 0 || height == 0 || width > MAX_WINDOW_SIZE || height > MAX_WINDOW_SIZE {
        return Err(ProtocolError::InvalidWindowSize);
    }

    Ok(())
}

struct PayloadReader<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, ProtocolError> {
        let bytes = self.read_slice(1)?;

        Ok(bytes[0])
    }

    fn read_u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes = self.read_slice(4)?;

        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes = self.read_slice(8)?;

        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_slice(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ProtocolError::InvalidPayload)?;

        if end > self.payload.len() {
            return Err(ProtocolError::InvalidPayload);
        }

        let bytes = &self.payload[self.offset..end];

        self.offset = end;

        Ok(bytes)
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.offset != self.payload.len() {
            return Err(ProtocolError::InvalidPayload);
        }

        Ok(())
    }
}
