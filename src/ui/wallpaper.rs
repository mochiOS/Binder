use viewkit::draw_command::ImageSampling;
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

const SETTINGS_PATH: &str = "/libraries/system/settings.conf";
const WALLPAPER_PATHS: [&str; 2] = [
    "/libraries/wallpapers/default.png",
    "/libraries/wallpapers/default.jpeg",
];

#[derive(Clone, Default)]
pub(crate) struct Wallpaper {
    image: Option<ImageData>,
}

impl Wallpaper {
    pub(crate) fn load_default_image() -> Option<ImageData> {
        if let Some(path) = configured_wallpaper_path()
            && let Ok(image) = ImageData::from_path(path)
        {
            return Some(image);
        }
        for path in WALLPAPER_PATHS {
            if let Ok(image) = ImageData::from_path(path) {
                return Some(image);
            }
        }
        None
    }

    pub(crate) fn load_default() -> Self {
        Self {
            image: Self::load_default_image(),
        }
    }
}

fn configured_wallpaper_path() -> Option<String> {
    let text = std::fs::read_to_string(SETTINGS_PATH).ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        (key.trim() == "wallpaper" && !value.is_empty()).then(|| value.to_owned())
    })
}

impl View for Wallpaper {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.maximum
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let Some(image) = self.image.clone() else {
            return;
        };
        Image::new(image)
            .content_mode(ImageContentMode::Fill)
            .sampling(ImageSampling::Bicubic)
            .paint(bounds, context);
    }
}
