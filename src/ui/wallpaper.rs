use viewkit::draw_command::ImageSampling;
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

const WALLPAPER_PATHS: [&str; 2] = [
    "/libraries/wallpapers/default.png",
    "/libraries/wallpapers/default.jpeg",
];

#[derive(Clone, Default)]
pub(crate) struct Wallpaper {
    image: Option<ImageData>,
}

impl Wallpaper {
    pub(crate) fn load_default() -> Self {
        for path in WALLPAPER_PATHS {
            if let Ok(image) = ImageData::from_path(path) {
                return Self { image: Some(image) };
            }
        }
        Self::default()
    }
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
