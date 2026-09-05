use std::fmt;

use mochi_user_syscall as syscall;
use viewkit::draw_command::DisplayList;
use viewkit::prelude::*;
use viewkit::typography::{TextMeasurer, Typography};
use viewkit::view::PaintContext;

use crate::ui::window_decoration::{TITLE_BAR_HEIGHT as VIEW_TITLE_BAR_HEIGHT, WindowDecoration};
use crate::window::{WindowControl, WindowInteraction};

const COMPOSITOR_SERVICE_NAME: &str = "compositor.service";
const OP_ATTACH_BUFFER: u32 = 2;
const OP_DAMAGE: u32 = 3;
const OP_COMMIT: u32 = 4;
const OP_DECOR_SUBSCRIBE: u32 = 100;
const OP_DECOR_CREATE_SURFACE: u32 = 101;
const OP_DECOR_ATTACH: u32 = 102;
const OP_DECOR_BEGIN_MOVE: u32 = 105;
const OP_DECOR_MINIMIZE: u32 = 107;
const OP_DECOR_TOGGLE_MAXIMIZE: u32 = 108;
const OP_DECOR_CLOSE_REQUEST: u32 = 109;
const DECOR_EVENT_WINDOW: u32 = 0x5749_4e44;
const DECOR_EVENT_POINTER_BUTTON: u32 = 0x4e54_4244;
const DECOR_EVENT_POINTER_MOTION: u32 = 0x544f_4d50;
const DECOR_EVENT_POINTER_LEAVE: u32 = 0x5641_454c;
const PIXEL_FORMAT_GPU_SCENE: u32 = 3;
const TITLE_BAR_HEIGHT: u32 = VIEW_TITLE_BAR_HEIGHT as u32;
const CONTROL_WIDTH: u32 = 44;
const PAGE_SIZE: usize = 4096;
const MAX_WINDOW_DIMENSION: u32 = 4096;
const POINTER_FLAG_PRESS: u32 = 1;
const POINTER_FLAG_RELEASE: u32 = 2;

#[derive(Debug)]
pub(super) struct DecorationError(u64);

impl fmt::Display for DecorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "errno={}", self.0)
    }
}

struct Decoration {
    window: u64,
    endpoint: u64,
    width: u32,
    surface: u64,
    buffer_virt: u64,
    buffer_capacity: usize,
    title: String,
    interaction: WindowInteraction,
}

pub(super) struct DecorationManager {
    compositor: u64,
    metadata_endpoint: u64,
    decorations: Vec<Decoration>,
}

impl DecorationManager {
    pub(super) fn connect() -> Result<Self, DecorationError> {
        let compositor = find_compositor()?;
        let metadata_endpoint = ipc_create()?;
        let mut request = [0u8; 12];
        put_u32(&mut request, 0, OP_DECOR_SUBSCRIBE)?;
        put_u64(&mut request, 4, metadata_endpoint)?;
        ipc_call_status(compositor, &request)?;
        Ok(Self {
            compositor,
            metadata_endpoint,
            decorations: Vec::new(),
        })
    }

    pub(super) fn handle_message(&mut self, event: &[u8]) -> Result<bool, DecorationError> {
        match read_u32(event, 0) {
            Some(DECOR_EVENT_WINDOW) => {
                if let Some((window, width, title)) = parse_window_event(event) {
                    if let Some(decoration) = self
                        .decorations
                        .iter_mut()
                        .find(|decoration| decoration.window == window)
                    {
                        if decoration.width != width {
                            let (buffer_virt, buffer_capacity) = attach_title_bar_buffer(
                                self.compositor,
                                decoration.surface,
                                width,
                                title.clone(),
                                decoration.interaction,
                            )?;
                            decoration.buffer_virt = buffer_virt;
                            decoration.buffer_capacity = buffer_capacity;
                            token_request(self.compositor, OP_DAMAGE, decoration.surface)?;
                            token_request(self.compositor, OP_COMMIT, decoration.surface)?;
                            decoration.width = width;
                            decoration.title = title;
                        } else if decoration.title != title {
                            decoration.title = title;
                            redraw_title_bar(self.compositor, decoration)?;
                        }
                    } else {
                        self.decorations.push(create_decoration(
                            self.compositor,
                            window,
                            width,
                            title,
                        )?);
                    }
                }
                Ok(true)
            }
            Some(DECOR_EVENT_POINTER_BUTTON) => {
                let window = read_u64(event, 16).ok_or(DecorationError(5))?;
                if let Some(decoration) = self
                    .decorations
                    .iter_mut()
                    .find(|decoration| decoration.window == window)
                {
                    handle_decoration_button(self.compositor, decoration, event)?;
                }
                Ok(true)
            }
            Some(DECOR_EVENT_POINTER_MOTION) => {
                let window = read_u64(event, 16).ok_or(DecorationError(5))?;
                if let Some(decoration) = self
                    .decorations
                    .iter_mut()
                    .find(|decoration| decoration.window == window)
                {
                    handle_decoration_motion(self.compositor, decoration, event)?;
                }
                Ok(true)
            }
            Some(DECOR_EVENT_POINTER_LEAVE) => {
                let window = read_u64(event, 16).ok_or(DecorationError(5))?;
                if let Some(decoration) = self
                    .decorations
                    .iter_mut()
                    .find(|decoration| decoration.window == window)
                    && decoration.interaction != WindowInteraction::default()
                {
                    decoration.interaction = WindowInteraction::default();
                    redraw_title_bar(self.compositor, decoration)?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn find_compositor() -> Result<u64, DecorationError> {
    let name = COMPOSITOR_SERVICE_NAME.as_bytes();
    let endpoint = syscall_result(syscall::call2(
        syscall::SyscallNumber::FindProcessByName,
        name.as_ptr() as u64,
        name.len() as u64,
    ))?;
    (endpoint != 0)
        .then_some(endpoint)
        .ok_or(DecorationError(2))
}

fn ipc_create() -> Result<u64, DecorationError> {
    let endpoint = syscall_result(syscall::call2(syscall::SyscallNumber::IpcCreate, 0, 0))?;
    (endpoint != 0)
        .then_some(endpoint)
        .ok_or(DecorationError(5))
}

fn parse_window_event(event: &[u8]) -> Option<(u64, u32, String)> {
    if read_u32(event, 0)? != DECOR_EVENT_WINDOW {
        return None;
    }
    let window = read_u64(event, 4)?;
    let width = read_u32(event, 12)?;
    let height = read_u32(event, 16)?;
    let title_length = read_u32(event, 44)? as usize;
    let title = std::str::from_utf8(event.get(48..48 + title_length)?)
        .ok()?
        .to_owned();
    (window != 0
        && width != 0
        && height != 0
        && width <= MAX_WINDOW_DIMENSION
        && height <= MAX_WINDOW_DIMENSION)
        .then_some((window, width, title))
}

fn create_decoration(
    compositor: u64,
    window: u64,
    width: u32,
    title: String,
) -> Result<Decoration, DecorationError> {
    let endpoint = ipc_create()?;
    let mut create = [0u8; 28];
    put_u32(&mut create, 0, OP_DECOR_CREATE_SURFACE)?;
    put_u64(&mut create, 4, window)?;
    put_u32(&mut create, 12, width)?;
    put_u32(&mut create, 16, TITLE_BAR_HEIGHT)?;
    put_u64(&mut create, 20, endpoint)?;
    let reply = ipc_call(compositor, &create)?;
    let surface = read_u64(&reply, 4).ok_or(DecorationError(5))?;
    let interaction = WindowInteraction::default();
    let (buffer_virt, buffer_capacity) =
        attach_title_bar_buffer(compositor, surface, width, title.clone(), interaction)?;
    token_request(compositor, OP_DAMAGE, surface)?;

    let mut attach = [0u8; 36];
    put_u32(&mut attach, 0, OP_DECOR_ATTACH)?;
    put_u64(&mut attach, 4, window)?;
    put_u64(&mut attach, 12, surface)?;
    put_u32(&mut attach, 24, TITLE_BAR_HEIGHT)?;
    ipc_call_status(compositor, &attach)?;
    Ok(Decoration {
        window,
        endpoint,
        width,
        surface,
        buffer_virt,
        buffer_capacity,
        title,
        interaction,
    })
}

fn attach_title_bar_buffer(
    compositor: u64,
    surface: u64,
    width: u32,
    title: String,
    interaction: WindowInteraction,
) -> Result<(u64, usize), DecorationError> {
    let rendered = render_title_bar(title, width, interaction)?;
    let pixel_bytes = (width as usize)
        .checked_mul(TITLE_BAR_HEIGHT as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(DecorationError(22))?;
    let byte_len = rendered.len().max(pixel_bytes);
    let page_count = byte_len
        .checked_add(PAGE_SIZE - 1)
        .map(|length| length / PAGE_SIZE)
        .ok_or(DecorationError(22))?;
    let virt = syscall_result(syscall::call4(
        syscall::SyscallNumber::AllocSharedPages,
        page_count as u64,
        0,
        0,
        0,
    ))?;
    if virt == 0 || virt & (PAGE_SIZE as u64 - 1) != 0 {
        return Err(DecorationError(5));
    }
    let capacity = page_count
        .checked_mul(PAGE_SIZE)
        .ok_or(DecorationError(22))?;
    let bytes = unsafe { std::slice::from_raw_parts_mut(virt as *mut u8, capacity) };
    bytes[..rendered.len()].copy_from_slice(&rendered);

    let mut attach = [0u8; 28];
    put_u32(&mut attach, 0, OP_ATTACH_BUFFER)?;
    put_u64(&mut attach, 4, surface)?;
    put_u32(&mut attach, 12, width)?;
    put_u32(&mut attach, 16, TITLE_BAR_HEIGHT)?;
    put_u32(&mut attach, 20, width)?;
    put_u32(&mut attach, 24, PIXEL_FORMAT_GPU_SCENE)?;
    ipc_call_status(compositor, &attach)?;
    syscall_result(syscall::call4(
        syscall::SyscallNumber::IpcSendPages,
        compositor,
        0,
        page_count as u64,
        virt,
    ))?;
    Ok((virt, capacity))
}

fn render_title_bar(
    title: String,
    width: u32,
    interaction: WindowInteraction,
) -> Result<Vec<u8>, DecorationError> {
    let decoration = WindowDecoration::new(title, interaction);
    let mut display_list = DisplayList::new();
    let mut text_measurer = TextMeasurer::new();
    let mut context = PaintContext::new(
        &mut display_list,
        &Theme::DEFAULT,
        &Typography::DEFAULT,
        &mut text_measurer,
    );
    decoration.paint(
        Rect::new(0.0, 0.0, width as f32, TITLE_BAR_HEIGHT as f32),
        &mut context,
    );
    viewkit::platform::mochios::render_offscreen_gpu_scene(
        &display_list,
        width,
        TITLE_BAR_HEIGHT,
        true,
    )
    .map_err(|_| DecorationError(5))
}

fn redraw_title_bar(compositor: u64, decoration: &Decoration) -> Result<(), DecorationError> {
    let rendered = render_title_bar(
        decoration.title.clone(),
        decoration.width,
        decoration.interaction,
    )?;
    if decoration.buffer_virt == 0 || rendered.len() > decoration.buffer_capacity {
        return Err(DecorationError(5));
    }
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(
            decoration.buffer_virt as *mut u8,
            decoration.buffer_capacity,
        )
    };
    bytes[..rendered.len()].copy_from_slice(&rendered);
    token_request(compositor, OP_DAMAGE, decoration.surface)?;
    token_request(compositor, OP_COMMIT, decoration.surface)
}

fn control_at(decoration: &Decoration, x: i32, y: i32) -> Option<WindowControl> {
    if x < 0 || y < 0 || y >= TITLE_BAR_HEIGHT as i32 || x >= decoration.width as i32 {
        return None;
    }
    let x = x as u32;
    if x >= decoration.width.saturating_sub(CONTROL_WIDTH) {
        Some(WindowControl::Close)
    } else if x >= decoration.width.saturating_sub(CONTROL_WIDTH * 2) {
        Some(WindowControl::Maximize)
    } else if x >= decoration.width.saturating_sub(CONTROL_WIDTH * 3) {
        Some(WindowControl::Minimize)
    } else {
        None
    }
}

fn handle_decoration_motion(
    compositor: u64,
    decoration: &mut Decoration,
    event: &[u8],
) -> Result<(), DecorationError> {
    let x = read_i32(event, 4).ok_or(DecorationError(5))?;
    let y = read_i32(event, 8).ok_or(DecorationError(5))?;
    let hovered = control_at(decoration, x, y);
    if decoration.interaction.hovered != hovered {
        decoration.interaction.hovered = hovered;
        redraw_title_bar(compositor, decoration)?;
    }
    Ok(())
}

fn handle_decoration_button(
    compositor: u64,
    decoration: &mut Decoration,
    event: &[u8],
) -> Result<(), DecorationError> {
    if read_u32(event, 0) != Some(DECOR_EVENT_POINTER_BUTTON) {
        return Ok(());
    }
    let detail = read_u32(event, 12).ok_or(DecorationError(5))?;
    let flags = detail >> 16;
    let x = read_i32(event, 4).ok_or(DecorationError(5))?;
    let y = read_i32(event, 8).ok_or(DecorationError(5))?;
    let control = control_at(decoration, x, y);
    if flags & POINTER_FLAG_PRESS != 0 {
        if let Some(control) = control {
            decoration.interaction.hovered = Some(control);
            decoration.interaction.pressed = Some(control);
            return redraw_title_bar(compositor, decoration);
        }
        let serial = read_u64(event, 24)
            .filter(|serial| *serial != 0)
            .ok_or(DecorationError(5))?;
        let mut request = [0u8; 28];
        put_u32(&mut request, 0, OP_DECOR_BEGIN_MOVE)?;
        put_u64(&mut request, 4, decoration.window)?;
        put_u64(&mut request, 12, serial)?;
        return ipc_call_status(compositor, &request);
    }
    if flags & POINTER_FLAG_RELEASE == 0 {
        return Ok(());
    }
    let pressed = decoration.interaction.pressed;
    decoration.interaction.hovered = control;
    decoration.interaction.pressed = None;
    if pressed.is_some() {
        redraw_title_bar(compositor, decoration)?;
    }
    if pressed != control {
        return Ok(());
    }
    match control {
        Some(WindowControl::Close) => {
            token_request(compositor, OP_DECOR_CLOSE_REQUEST, decoration.window)
        }
        Some(WindowControl::Maximize) => {
            token_request(compositor, OP_DECOR_TOGGLE_MAXIMIZE, decoration.window)
        }
        Some(WindowControl::Minimize) => {
            token_request(compositor, OP_DECOR_MINIMIZE, decoration.window)
        }
        None => Ok(()),
    }
}

fn token_request(compositor: u64, opcode: u32, token: u64) -> Result<(), DecorationError> {
    let mut request = [0u8; 12];
    put_u32(&mut request, 0, opcode)?;
    put_u64(&mut request, 4, token)?;
    ipc_call_status(compositor, &request)
}

fn ipc_call_status(compositor: u64, request: &[u8]) -> Result<(), DecorationError> {
    ipc_call(compositor, request).map(|_| ())
}

fn ipc_call(compositor: u64, request: &[u8]) -> Result<[u8; 16], DecorationError> {
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
        return Err(DecorationError(5));
    }
    let status = read_u32(&reply, 0).ok_or(DecorationError(5))?;
    if status != 0 {
        return Err(DecorationError(status as u64));
    }
    Ok(reply)
}

fn syscall_result<T>(result: syscall::SysResult<T>) -> Result<T, DecorationError> {
    result.map_err(|error| DecorationError(error.errno().unwrap_or(5)))
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

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), DecorationError> {
    bytes
        .get_mut(offset..offset + 4)
        .ok_or(DecorationError(22))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) -> Result<(), DecorationError> {
    bytes
        .get_mut(offset..offset + 8)
        .ok_or(DecorationError(22))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}
