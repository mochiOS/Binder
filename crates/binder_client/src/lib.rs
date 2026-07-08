mod platform;
mod protocol;

use std::collections::VecDeque;
use std::fmt;

use protocol::{ApplicationId, ClientRequest, RemoteWindowId, ServerEvent};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowHandle(u64);

impl WindowHandle {
    pub fn raw(self) -> u64 {
        self.0
    }

    fn from_remote(window: RemoteWindowId) -> Self {
        Self(window.0)
    }

    fn to_remote(self) -> RemoteWindowId {
        RemoteWindowId(self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowOptions {
    title: String,
    width: u32,
    height: u32,
    resizable: bool,
}

impl WindowOptions {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            resizable: true,
        }
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_resizable(&self) -> bool {
        self.resizable
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    CloseRequested {
        window: WindowHandle,
    },

    Resized {
        window: WindowHandle,
        width: u32,
        height: u32,
    },

    FocusChanged {
        window: WindowHandle,
        focused: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    TransportUnavailable,
    UnsupportedPlatform,
    ConnectionFailed,
    ConnectionClosed,
    SendFailed,
    ReceiveFailed,
    ProtocolError,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransportUnavailable => {
                write!(formatter, "Binder transport is unavailable")
            }

            Self::UnsupportedPlatform => {
                write!(formatter, "Binder is unsupported on this platform")
            }

            Self::ConnectionFailed => {
                write!(formatter, "failed to connect to Binder")
            }

            Self::ConnectionClosed => {
                write!(formatter, "Binder closed the connection")
            }

            Self::SendFailed => {
                write!(formatter, "failed to send a Binder request")
            }

            Self::ReceiveFailed => {
                write!(formatter, "failed to receive a Binder event")
            }

            Self::ProtocolError => {
                write!(formatter, "Binder protocol error")
            }
        }
    }
}

impl std::error::Error for Error {}

pub struct BinderClient {
    transport: platform::Transport,
    pending_events: VecDeque<WindowEvent>,
}

impl BinderClient {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            transport: platform::Transport::connect()?,
            pending_events: VecDeque::new(),
        })
    }

    pub fn create_window(&mut self, options: WindowOptions) -> Result<WindowHandle> {
        self.create_window_for_application(ApplicationId::About, options)
    }

    pub fn create_test_window(&mut self, options: WindowOptions) -> Result<WindowHandle> {
        self.create_window_for_application(ApplicationId::Test, options)
    }

    fn create_window_for_application(
        &mut self,
        application: ApplicationId,
        options: WindowOptions,
    ) -> Result<WindowHandle> {
        self.transport.send(&ClientRequest::CreateWindow {
            application,
            title: options.title,
            width: options.width,
            height: options.height,
            resizable: options.resizable,
        })?;

        loop {
            match self.transport.receive()? {
                ServerEvent::WindowCreated { window } => {
                    return Ok(WindowHandle::from_remote(window));
                }

                event => {
                    if let Some(event) = convert_server_event(event) {
                        self.pending_events.push_back(event);
                    }
                }
            }
        }
    }

    pub fn close_window(&mut self, window: WindowHandle) -> Result<()> {
        self.transport.send(&ClientRequest::CloseWindow {
            window: window.to_remote(),
        })
    }

    pub fn next_event(&mut self) -> Result<WindowEvent> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }

        loop {
            let event = self.transport.receive()?;

            if let Some(event) = convert_server_event(event) {
                return Ok(event);
            }
        }
    }
}

fn convert_server_event(event: ServerEvent) -> Option<WindowEvent> {
    match event {
        ServerEvent::WindowCreated { .. } => None,

        ServerEvent::CloseRequested { window } => Some(WindowEvent::CloseRequested {
            window: WindowHandle::from_remote(window),
        }),

        ServerEvent::Resized {
            window,
            width,
            height,
        } => Some(WindowEvent::Resized {
            window: WindowHandle::from_remote(window),
            width,
            height,
        }),

        ServerEvent::FocusChanged { window, focused } => Some(WindowEvent::FocusChanged {
            window: WindowHandle::from_remote(window),
            focused,
        }),
    }
}
