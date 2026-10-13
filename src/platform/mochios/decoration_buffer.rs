use super::{DecorationError, platform};
use platform::memory_object::{MemoryMapping, MemoryObject};

pub(super) struct TitleBarBuffer {
    mapping: MemoryMapping,
    object: MemoryObject,
    pixels: usize,
    capacity: usize,
}

impl TitleBarBuffer {
    pub(super) fn new(pixels: usize) -> Result<Self, DecorationError> {
        let capacity = pixels
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(4095))
            .map(|bytes| bytes & !4095)
            .filter(|bytes| *bytes != 0)
            .ok_or(DecorationError(22))?;
        let object = MemoryObject::create(capacity as u64)
            .map_err(|error| DecorationError(error.errno().unwrap_or(5)))?;
        let mapping = object
            .map(
                0,
                capacity as u64,
                platform::syscall::MEMORY_ACCESS_READ | platform::syscall::MEMORY_ACCESS_WRITE,
            )
            .map_err(|error| DecorationError(error.errno().unwrap_or(5)))?;
        Ok(Self {
            mapping,
            object,
            pixels,
            capacity,
        })
    }

    pub(super) fn write(&mut self, pixels: &[u32]) -> Result<(), DecorationError> {
        if pixels.len() != self.pixels {
            return Err(DecorationError(22));
        }
        // The compositor aliases this RAM. Do not form an exclusive Rust
        // slice over it; the rendered source pixels are separately owned.
        unsafe {
            core::ptr::copy_nonoverlapping(
                pixels.as_ptr(),
                self.mapping.as_mut_ptr().cast::<u32>(),
                pixels.len(),
            );
        }
        Ok(())
    }

    pub(super) fn offer(&self, endpoint: u64) -> Result<(), DecorationError> {
        self.object
            .send_readonly(endpoint, 0, self.capacity as u64)
            .map_err(|error| DecorationError(error.errno().unwrap_or(5)))
    }
}
