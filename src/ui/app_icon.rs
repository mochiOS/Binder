//! Application icon presentation owned by the desktop, using ViewKit's
//! continuous-corner image clipping rather than a separate mask implementation.

use viewkit::{draw_command::ImageSampling, prelude::*, view::PaintContext};

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
