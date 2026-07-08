use std::io::{Read, Write};

use std::os::unix::net::UnixStream;

use crate::Error;

use crate::protocol::{ClientRequest, ServerEvent, encode_client_request, try_decode_server_event};

const BINDER_SOCKET_ENV: &str = "BINDER_SOCKET";

pub(crate) struct Transport {
    stream: UnixStream,
    read_buffer: Vec<u8>,
}

impl Transport {
    pub(crate) fn connect() -> Result<Self, Error> {
        let socket_path = std::env::var_os(BINDER_SOCKET_ENV).ok_or(Error::TransportUnavailable)?;

        let stream = UnixStream::connect(socket_path).map_err(|_| Error::ConnectionFailed)?;

        Ok(Self {
            stream,
            read_buffer: Vec::new(),
        })
    }

    pub(crate) fn send(&mut self, request: &ClientRequest) -> Result<(), Error> {
        let frame = encode_client_request(request).map_err(|_| Error::ProtocolError)?;

        self.stream
            .write_all(&frame)
            .map_err(|_| Error::SendFailed)?;

        self.stream.flush().map_err(|_| Error::SendFailed)?;

        Ok(())
    }

    pub(crate) fn receive(&mut self) -> Result<ServerEvent, Error> {
        loop {
            if let Some(event) =
                try_decode_server_event(&mut self.read_buffer).map_err(|_| Error::ProtocolError)?
            {
                return Ok(event);
            }

            let mut buffer = [0_u8; 1024];

            let read = self
                .stream
                .read(&mut buffer)
                .map_err(|_| Error::ReceiveFailed)?;

            if read == 0 {
                return Err(Error::ConnectionClosed);
            }

            self.read_buffer.extend_from_slice(&buffer[..read]);
        }
    }
}
