//! Application icon presentation owned by the desktop, using ViewKit's
//! continuous-corner image clipping rather than a separate mask implementation.

use std::{cell::RefCell, collections::HashMap, path::PathBuf};
use viewkit::{draw_command::ImageSampling, prelude::*, view::PaintContext};

#[derive(Clone)]
enum CachedIcon {
    Raster(ImageData),
    Missing,
}

#[derive(Default)]
pub(super) struct AppIcons(RefCell<HashMap<PathBuf, CachedIcon>>);

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
        if !self.0.borrow().contains_key(path) {
            let icon = if path.extension().is_some_and(|extension| extension == "svg") {
                SvgData::from_path(path)
                    .ok()
                    .and_then(|svg| ImageData::from_svg(&svg, 128, 128).ok())
                    .map(CachedIcon::Raster)
                    .unwrap_or(CachedIcon::Missing)
            } else {
                ImageData::thumbnail_from_path(path, 128, 128)
                    .map(CachedIcon::Raster)
                    .unwrap_or(CachedIcon::Missing)
            };
            self.0.borrow_mut().insert(path.clone(), icon);
        }
        match self
            .0
            .borrow()
            .get(path)
            .cloned()
            .unwrap_or(CachedIcon::Missing)
        {
            CachedIcon::Raster(data) => paint(data, bounds, context),
            CachedIcon::Missing => super::app_library::paint_fallback_icon(bounds, context),
        }
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
