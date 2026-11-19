use std::io;
use viewkit::prelude::{Point, Rect, Size};

const FILE: &str = "tablet-flick-placement-v1";
const MAX_BYTES: usize = 128;
const MIN_SCALE: f32 = 0.75;
const MAX_SCALE: f32 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            x: 1.0,
            y: 1.0,
            scale: 1.0,
        }
    }
}

impl Placement {
    pub fn load() -> Self {
        match read() {
            Ok(Some(bytes)) => Self::decode(&bytes).unwrap_or_default(),
            Ok(None) => Self::default(),
            Err(error) => {
                use std::io::Write;
                let _ = writeln!(
                    io::stderr(),
                    "Binder.app: could not read floating keyboard placement: {error}"
                );
                Self::default()
            }
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut values = std::str::from_utf8(bytes).ok()?.split_whitespace();
        let x = values.next()?.parse().ok()?;
        let y = values.next()?.parse().ok()?;
        let scale = values.next()?.parse().ok()?;
        (values.next().is_none()
            && (0.0..=1.0).contains(&x)
            && (0.0..=1.0).contains(&y)
            && (MIN_SCALE..=MAX_SCALE).contains(&scale))
        .then_some(Self { x, y, scale })
    }

    pub fn save(self) -> io::Result<()> {
        write(format!("{} {} {}\n", self.x, self.y, self.scale).as_bytes())
    }
}

#[cfg(target_os = "mochios")]
fn read() -> io::Result<Option<Vec<u8>>> {
    appcore::ApplicationData::open()?.read(FILE, MAX_BYTES)
}

#[cfg(target_os = "mochios")]
fn write(bytes: &[u8]) -> io::Result<()> {
    appcore::ApplicationData::open()?.write(FILE, bytes)
}

#[cfg(not(target_os = "mochios"))]
fn preview_path() -> io::Result<std::path::PathBuf> {
    use std::path::PathBuf;
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|root| root.join("mochios/Binder.app").join(FILE))
        .ok_or_else(|| io::ErrorKind::NotFound.into())
}

#[cfg(not(target_os = "mochios"))]
fn read() -> io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    match std::fs::File::open(preview_path()?) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > MAX_BYTES {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(not(target_os = "mochios"))]
fn write(bytes: &[u8]) -> io::Result<()> {
    let path = preview_path()?;
    std::fs::create_dir_all(path.parent().ok_or(io::ErrorKind::NotFound)?)?;
    let temporary = path.with_extension(format!("{}.new", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

#[derive(Clone, Copy)]
pub(super) struct FloatingFrame {
    pub area: Rect,
    pub header: f32,
    pub base: Size,
}

impl FloatingFrame {
    fn scale(self, scale: f32) -> f32 {
        let max = MAX_SCALE
            .min(self.area.size.width / self.base.width)
            .min((self.area.size.height - self.header) / self.base.height);
        scale.clamp(MIN_SCALE.min(max), max)
    }

    fn extent(self, scale: f32) -> Size {
        let scale = self.scale(scale);
        Size::new(
            self.base.width * scale,
            self.base.height * scale + self.header,
        )
    }

    pub fn bounds(self, placement: Placement) -> Rect {
        let size = self.extent(placement.scale);
        Rect::new(
            self.area.origin.x + (self.area.size.width - size.width).max(0.0) * placement.x,
            self.area.origin.y + (self.area.size.height - size.height).max(0.0) * placement.y,
            size.width,
            size.height,
        )
    }

    pub fn adjust(
        self,
        initial: Placement,
        panel: Rect,
        delta: Point,
        resizing: bool,
    ) -> Placement {
        let mut placement = initial;
        let origin = if resizing {
            // Project the pointer movement onto the keyboard's diagonal.
            // The bottom-right stays anchored until the frame reaches an edge.
            let change = (delta.x * self.base.width + delta.y * self.base.height)
                / (self.base.width.powi(2) + self.base.height.powi(2));
            placement.scale = self
                .scale(self.scale(initial.scale) - change)
                .clamp(MIN_SCALE, MAX_SCALE);
            let size = self.extent(placement.scale);
            Point::new(
                panel.origin.x + panel.size.width - size.width,
                panel.origin.y + panel.size.height - size.height,
            )
        } else {
            Point::new(panel.origin.x + delta.x, panel.origin.y + delta.y)
        };
        let extent = self.extent(placement.scale);
        let travel_x = self.area.size.width - extent.width;
        let travel_y = self.area.size.height - extent.height;
        if travel_x > 0.0 {
            placement.x = ((origin.x - self.area.origin.x) / travel_x).clamp(0.0, 1.0);
        }
        if travel_y > 0.0 {
            placement.y = ((origin.y - self.area.origin.y) / travel_y).clamp(0.0, 1.0);
        }
        placement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_position_stays_reachable_after_rotation() {
        assert!(Placement::decode(b"NaN 1 1").is_none());
        assert!(Placement::decode(b"0 1 10").is_none());
        let mut frame = FloatingFrame {
            area: Rect::new(12., 50., 1896., 964.),
            header: 44.,
            base: Size::new(374., 307.),
        };
        let initial = Placement::default();
        let panel = frame.bounds(initial);
        let larger = frame.adjust(initial, panel, Point::new(-120., -100.), true);
        assert!(larger.scale > initial.scale);
        let enlarged = frame.bounds(larger);
        assert_eq!(
            panel.origin.x + panel.size.width,
            enlarged.origin.x + enlarged.size.width
        );
        let moved = frame.adjust(larger, enlarged, Point::new(-2000., -2000.), false);
        assert_eq!(
            moved,
            Placement {
                x: 0.,
                y: 0.,
                scale: larger.scale
            }
        );
        frame.area = Rect::new(10., 40., 300., 500.);
        let reflowed = frame.bounds(moved);
        assert_eq!(reflowed.origin, frame.area.origin);
        assert!(reflowed.size.width <= frame.area.size.width);
        assert!(reflowed.size.height <= frame.area.size.height);
    }
}
