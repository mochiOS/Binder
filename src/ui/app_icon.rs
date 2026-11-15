//! Application icon presentation owned by the desktop, using ViewKit's
//! continuous-corner image clipping rather than a separate mask implementation.

use std::sync::OnceLock;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

const FALLBACK_APP_ICON_SVG: &[u8] = include_bytes!("../../resources/appicon.svg");

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
            return paint_fallback_icon(bounds, context);
        };
        if let Some(data) = self.load(path, bounds, context.scale_factor()) {
            paint(data, bounds, context);
        } else {
            paint_fallback_icon(bounds, context);
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

pub(super) fn fallback_app_icon() -> Option<&'static SvgData> {
    static ICON: OnceLock<Option<SvgData>> = OnceLock::new();
    ICON.get_or_init(|| SvgData::decode(FALLBACK_APP_ICON_SVG).ok())
        .as_ref()
}

pub(super) fn paint_fallback_icon(bounds: Rect, context: &mut PaintContext<'_>) {
    if let Some(icon) = fallback_app_icon() {
        Svg::new(icon.clone())
            .radius(radius(bounds))
            .paint(bounds, context);
        return;
    }

    Rectangle::new()
        .color(RectangleColor::Custom(context.theme.colors.surface_muted))
        .radius(radius(bounds))
        .border(BorderStyle::Standard { width: 1.0 })
        .paint(bounds, context);
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
    fn absent_missing_and_invalid_app_icons_all_use_binders_own_artwork() {
        use viewkit::{draw_command::DisplayList, typography::TextMeasurer};
        let bounds = Rect::new(0., 0., 56., 56.);
        let theme = Theme::LIGHT;
        let mut text = TextMeasurer::new();
        let mut expected = DisplayList::new();
        paint_fallback_icon(
            bounds,
            &mut PaintContext::new(&mut expected, &theme, &theme.typography, &mut text),
        );
        assert!(fallback_app_icon().is_some());
        let icons = AppIcons::default();
        for icon in [
            None,
            Some(PathBuf::from("/nonexistent/mochios-missing-icon.png")),
            Some(PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/Cargo.toml"
            ))),
        ] {
            let app = crate::platform::AppInfo {
                root: PathBuf::new(),
                name: "No Artwork".into(),
                bundle_id: "org.example.no-artwork".into(),
                version: "1".into(),
                developer: String::new(),
                entry: String::new(),
                description: String::new(),
                icon,
                resources: Vec::new(),
            };
            let mut actual = DisplayList::new();
            icons.paint(
                &app,
                bounds,
                &mut PaintContext::new(&mut actual, &theme, &theme.typography, &mut text),
            );
            assert_eq!(actual.commands(), expected.commands());
        }
    }
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
