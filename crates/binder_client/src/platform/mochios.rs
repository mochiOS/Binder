use crate::Error;

use crate::protocol::{ClientRequest, ServerEvent};

pub(crate) struct Transport;

impl Transport {
    pub(crate) fn connect() -> Result<Self, Error> {
        Err(Error::UnsupportedPlatform)
    }

    pub(crate) fn send(&mut self, _request: &ClientRequest) -> Result<(), Error> {
        Err(Error::UnsupportedPlatform)
    }

    pub(crate) fn receive(&mut self) -> Result<ServerEvent, Error> {
        Err(Error::UnsupportedPlatform)
    }
}
