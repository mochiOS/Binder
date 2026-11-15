//! Application icon presentation owned by the desktop, using ViewKit's
//! continuous-corner image clipping rather than a separate mask implementation.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};
use viewkit::{draw_command::ImageSampling, prelude::*, view::PaintContext};

#[derive(Clone)]
enum CachedIcon {
    Raster(ImageData),
    Missing,
}

#[derive(Default)]
pub(super) struct AppIcons(RefCell<Vec<((PathBuf, u32), CachedIcon)>>);

fn raster_size(bounds: Rect, scale: f64) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let size = (bounds.size.width.max(bounds.size.height) as f64 * scale)
        .ceil()
        .clamp(16.0, 1024.0) as u32;
    // Stable 16px buckets avoid reallocating an SVG on every magnification frame.
    size.div_ceil(16) * 16
}

impl AppIcons {
    pub fn paint(
        &self,
        app: &crate::platform::AppInfo,
        bounds: Rect,
        context: &mut PaintContext<'_>,
    ) {
        let Some(path) = &app.icon else {
            return super::app_library::paint_fallback_icon(bounds, context);
        };
        if let Some(data) = self.load(path, bounds, context.scale_factor()) {
            paint(data, bounds, context);
        } else {
            super::app_library::paint_fallback_icon(bounds, context);
        }
    }

    /// Loads a bounded, physical-size raster shared by desktop presentations.
    pub(super) fn load(&self, path: &Path, bounds: Rect, scale: f64) -> Option<ImageData> {
        let size = raster_size(bounds, scale);
        let key = (path.to_path_buf(), size);
        let mut cache = self.0.borrow_mut();
        if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(index);
            let result = match &entry.1 {
                CachedIcon::Raster(data) => Some(data.clone()),
                CachedIcon::Missing => None,
            };
            cache.push(entry);
            return result;
        }
        let data = if path.extension().is_some_and(|e| e == "svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| ImageData::from_svg(&svg, size, size).ok())
        } else {
            ImageData::thumbnail_from_path(path, size, size).ok()
        };
        let cached = data
            .clone()
            .map(CachedIcon::Raster)
            .unwrap_or(CachedIcon::Missing);
        let bytes = |icon: &CachedIcon| match icon {
            CachedIcon::Raster(image) => image.premultiplied_rgba8().len(),
            CachedIcon::Missing => 0,
        };
        let cost = bytes(&cached);
        let mut used: usize = cache.iter().map(|(_, icon)| bytes(icon)).sum();
        while !cache.is_empty() && (cache.len() >= 64 || used + cost > 16 * 1024 * 1024) {
            used = used.saturating_sub(bytes(&cache.remove(0).1));
        }
        cache.push((key, cached));
        data
    }
}

const RADIUS_RATIO: f32 = 0.30;

/// アイコンの表示サイズに比例する連続角丸の半径を返す。
pub(super) fn radius(bounds: Rect) -> CornerRadius {
    CornerRadius::Custom(bounds.size.width.min(bounds.size.height).max(0.0) * RADIUS_RATIO)
}

/// 同じマスクと補間規則でアプリアイコンを描画する。
pub(super) fn paint(image: ImageData, bounds: Rect, context: &mut PaintContext<'_>) {
    Image::new(image)
        .content_mode(ImageContentMode::Fit)
        .sampling(ImageSampling::Bicubic)
        .radius(radius(bounds))
        .paint(bounds, context);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_buckets_follow_physical_size_without_unbounded_growth() {
        let r = Rect::new(0., 0., 44., 44.);
        assert_eq!(raster_size(r, 1.), 48);
        assert_eq!(raster_size(r, 2.), 96);
        assert_eq!(raster_size(Rect::new(0., 0., 99999., 99999.), 2.), 1024);
        assert_eq!(raster_size(r, f64::NAN), 48);
        let shape = radius(r).resolve(&Theme::LIGHT.radius, 44., 44.);
        assert!((shape - 13.2).abs() < 0.001);
    }
}
