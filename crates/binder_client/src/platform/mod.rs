#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
mod mochios;

#[cfg(target_os = "linux")]
pub(crate) use linux::Transport;

#[cfg(not(target_os = "linux"))]
pub(crate) use mochios::Transport;
