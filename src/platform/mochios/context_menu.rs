use std::fmt;

use mochi_user_syscall as syscall;
use viewkit::prelude::{Point, State};

use crate::platform::{ContextMenuEntry, ContextMenuModel};

const COMPOSITOR_SERVICE_NAME: &str = "compositor.service";
const OP_CONTEXT_MENU_SUBSCRIBE: u32 = 120;
const OP_CONTEXT_MENU_COMPLETE: u32 = 122;
const CONTEXT_MENU_EVENT_SHOW: u32 = 0x554e_454d;
const CONTEXT_MENU_EVENT_DISMISS: u32 = 0x5349_444d;
const MAX_ITEMS: u32 = 32;
const MAX_LABEL_BYTES: usize = 128;

#[derive(Debug)]
pub(super) struct ContextMenuError(u64);

impl fmt::Display for ContextMenuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "errno={}", self.0)
    }
}

pub(super) struct ContextMenuManager {
    compositor: u64,
}

impl ContextMenuManager {
    pub(super) fn connect() -> Result<Self, ContextMenuError> {
        let compositor = find_compositor()?;
        let endpoint = syscall_result(syscall::call2(syscall::SyscallNumber::IpcCreate, 0, 0))?;
        if endpoint == 0 {
            return Err(ContextMenuError(5));
        }
        let mut request = [0u8; 12];
        request[0..4].copy_from_slice(&OP_CONTEXT_MENU_SUBSCRIBE.to_le_bytes());
        request[4..12].copy_from_slice(&endpoint.to_le_bytes());
        ipc_call_status(compositor, &request)?;
        Ok(Self { compositor })
    }

    pub(super) fn handle_message(
        &mut self,
        message: &[u8],
        state: &State<Option<ContextMenuModel>>,
    ) -> Result<bool, ContextMenuError> {
        match read_u32(message, 0) {
            Some(CONTEXT_MENU_EVENT_SHOW) => {
                let model = parse_show(message).ok_or(ContextMenuError(22))?;
                state.set(Some(model));
                Ok(true)
            }
            Some(CONTEXT_MENU_EVENT_DISMISS) => {
                let request_id = read_u64(message, 8).ok_or(ContextMenuError(22))?;
                if state
                    .get()
                    .as_ref()
                    .is_some_and(|model| model.request_id == request_id)
                {
                    self.complete(request_id, None)?;
                    state.set(None);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn complete(
        &mut self,
        request_id: u64,
        command_id: Option<u32>,
    ) -> Result<(), ContextMenuError> {
        if request_id == 0 {
            return Err(ContextMenuError(22));
        }
        let mut request = [0u8; 24];
        request[0..4].copy_from_slice(&OP_CONTEXT_MENU_COMPLETE.to_le_bytes());
        request[4..8].copy_from_slice(&u32::from(command_id.is_none()).to_le_bytes());
        request[8..16].copy_from_slice(&request_id.to_le_bytes());
        request[16..20].copy_from_slice(&command_id.unwrap_or(0).to_le_bytes());
        ipc_call_status(self.compositor, &request)
    }
}

fn parse_show(message: &[u8]) -> Option<ContextMenuModel> {
    let x = read_i32(message, 4)?;
    let y = read_i32(message, 8)?;
    let item_count = read_u32(message, 12)?;
    let request_id = read_u64(message, 16)?;
    if request_id == 0 || item_count == 0 || item_count > MAX_ITEMS {
        return None;
    }
    let mut entries = Vec::with_capacity(item_count as usize);
    let mut offset = 24usize;
    for _ in 0..item_count {
        let command_id = read_u32(message, offset)?;
        let flags = read_u16(message, offset + 4)?;
        let label_len = read_u16(message, offset + 6)? as usize;
        if label_len > MAX_LABEL_BYTES {
            return None;
        }
        offset = offset.checked_add(8)?;
        let label = std::str::from_utf8(message.get(offset..offset.checked_add(label_len)?)?)
            .ok()?
            .to_owned();
        offset = offset.checked_add(label_len)?;
        entries.push(ContextMenuEntry {
            command_id,
            label,
            separator: flags & (1 << 0) != 0,
            enabled: flags & (1 << 1) != 0,
            checked: flags & (1 << 2) != 0,
            destructive: flags & (1 << 3) != 0,
        });
    }
    (offset == message.len()).then_some(ContextMenuModel {
        request_id,
        position: Point::new(x as f32, y as f32),
        entries,
    })
}

fn find_compositor() -> Result<u64, ContextMenuError> {
    let name = COMPOSITOR_SERVICE_NAME.as_bytes();
    let endpoint = syscall_result(syscall::call2(
        syscall::SyscallNumber::FindProcessByName,
        name.as_ptr() as u64,
        name.len() as u64,
    ))?;
    (endpoint != 0)
        .then_some(endpoint)
        .ok_or(ContextMenuError(2))
}

fn ipc_call_status(compositor: u64, request: &[u8]) -> Result<(), ContextMenuError> {
    let mut reply = [0u8; 16];
    let message = syscall_result(syscall::call5(
        syscall::SyscallNumber::IpcCall,
        compositor,
        request.as_ptr() as u64,
        request.len() as u64,
        reply.as_mut_ptr() as u64,
        reply.len() as u64,
    ))?;
    if (message & 0xffff_ffff) < 4 {
        return Err(ContextMenuError(5));
    }
    match read_u32(&reply, 0) {
        Some(0) => Ok(()),
        Some(status) => Err(ContextMenuError(status as u64)),
        None => Err(ContextMenuError(5)),
    }
}

fn syscall_result<T>(result: syscall::SysResult<T>) -> Result<T, ContextMenuError> {
    result.map_err(|error| ContextMenuError(error.errno().unwrap_or(5)))
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forwarded_menu() {
        let mut message = vec![0u8; 36];
        message[0..4].copy_from_slice(&CONTEXT_MENU_EVENT_SHOW.to_le_bytes());
        message[4..8].copy_from_slice(&40i32.to_le_bytes());
        message[8..12].copy_from_slice(&60i32.to_le_bytes());
        message[12..16].copy_from_slice(&1u32.to_le_bytes());
        message[16..24].copy_from_slice(&9u64.to_le_bytes());
        message[24..28].copy_from_slice(&7u32.to_le_bytes());
        message[28..30].copy_from_slice(&2u16.to_le_bytes());
        message[30..32].copy_from_slice(&4u16.to_le_bytes());
        message[32..36].copy_from_slice(b"Open");
        let model = parse_show(&message);
        assert!(model.is_some());
        let model = model.unwrap_or(ContextMenuModel {
            request_id: 0,
            position: Point::new(0.0, 0.0),
            entries: Vec::new(),
        });
        assert_eq!(model.request_id, 9);
        assert_eq!(model.entries[0].command_id, 7);
        assert!(model.entries[0].enabled);
    }
}
