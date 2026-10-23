//! Desktop chrome values from Desktop - Mock.svg, with requested adjustments.

pub(crate) const SYSTEM_HEIGHT: f32 = 54.0;
pub(crate) const SYSTEM_FOREGROUND: viewkit::theme::Color = viewkit::theme::Color::WHITE;
pub(crate) const LOGO_INSET: f32 = 15.0;
pub(crate) const LOGO_SIZE: f32 = 24.0;
pub(crate) const STATUS_RIGHT: f32 = 18.0;
pub(crate) const STATUS_WIDTH: f32 = 184.0;
pub(crate) const DOCK_HEIGHT: f32 = 89.0;
pub(crate) const DOCK_BOTTOM: f32 = 10.0;
pub(crate) const DOCK_PADDING: f32 = 12.5;
pub(crate) const DOCK_ICON: f32 = 64.0;
pub(crate) const DOCK_GAP: f32 = 10.0;
pub(crate) const DOCK_RADIUS: f32 = 26.5;
pub(crate) const DOCK_SEPARATOR_SPACE: f32 = 11.0;
pub(crate) const DOCK_SEPARATOR_HEIGHT: f32 = 48.0;
pub(crate) const DOCK_SEPARATOR_COLOR: viewkit::theme::Color =
    viewkit::theme::Color::rgb(123, 123, 123);

// Menu - Mock.png (1440 × 1024), independent of app control defaults.
pub(crate) const OS_MENU_TOP: f32 = 47.0;
pub(crate) const OS_MENU_WIDTH: f32 = 150.0;
pub(crate) const OS_MENU_HEIGHT: f32 = 144.0;
pub(crate) const STATUS_MENU_SPACE: f32 = 32.0;
pub(crate) const STATUS_GROUP_WIDTH: f32 = 224.0 + STATUS_MENU_SPACE * 2.0;
pub(crate) const STATUS_GROUP_TOP: f32 = 10.0;
pub(crate) const CHROME_RIGHT: f32 = 15.0;
pub(crate) const CONTROL_TOP: f32 = 54.0;
pub(crate) const CONTROL_SIZE: f32 = 60.0;
pub(crate) const CONTROL_GAP: f32 = 10.0;
pub(crate) const CONTROL_PITCH: f32 = CONTROL_SIZE + CONTROL_GAP;
pub(crate) const CONTROL_WIDTH: f32 = CONTROL_SIZE * 3.0 + CONTROL_GAP * 2.0;
pub(crate) const CONTROL_SLIDER_HEIGHT: f32 = CONTROL_SIZE * 2.0 + CONTROL_GAP;
