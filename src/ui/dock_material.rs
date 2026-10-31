//! Binder owns Dock appearance; ViewKit supplies only generic drawing primitives.

use viewkit::theme::{Color, Shadow, ShadowSet, Theme};

#[derive(Clone, Copy)]
pub(super) struct DockMaterial {
    pub(super) background: Color,
    pub(super) border: Color,
    pub(super) separator: Color,
    pub(super) item_hover: Color,
    pub(super) app_library_icon_background: Color,
    pub(super) app_library_icon_border: Color,
    pub(super) app_library_icon_tile: Color,
    pub(super) running_indicator: Color,
    pub(super) shadow: ShadowSet,
}

impl DockMaterial {
    const LIGHT: Self = Self {
        background: Color::rgba(255, 255, 255, 72),
        border: Color::rgba(0, 0, 0, 12),
        separator: Color::rgba(0, 0, 0, 40),
        item_hover: Color::rgba(255, 255, 255, 64),
        app_library_icon_background: Color::rgba(255, 255, 255, 208),
        app_library_icon_border: Color::rgba(0, 0, 0, 28),
        app_library_icon_tile: Color::rgba(255, 255, 255, 225),
        running_indicator: Color::rgba(45, 45, 45, 210),
        shadow: ShadowSet::single(Shadow::new(Color::rgba(0, 0, 0, 8), 0., 2., 8., 0.)),
    };

    const DARK: Self = Self {
        background: Color::rgba(44, 44, 46, 96),
        border: Color::rgba(255, 255, 255, 18),
        separator: Color::rgba(255, 255, 255, 48),
        item_hover: Color::rgba(255, 255, 255, 30),
        app_library_icon_background: Color::rgba(44, 44, 46, 208),
        app_library_icon_border: Color::rgba(255, 255, 255, 32),
        app_library_icon_tile: Color::rgba(255, 255, 255, 30),
        running_indicator: Color::rgba(245, 245, 247, 210),
        shadow: Self::LIGHT.shadow,
    };

    /// 現在のテーマの明暗に合うBinder専用のDock材質を返す。
    pub(super) fn for_theme(theme: &Theme) -> Self {
        let background = theme.colors.background;
        let brightness = u32::from(background.red) * 299
            + u32::from(background.green) * 587
            + u32::from(background.blue) * 114;
        if brightness < 128_000 {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }
}
