use std::sync::OnceLock;

use crate::window::{WindowControl, WindowInteraction};
use viewkit::{
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) const TITLE_BAR_HEIGHT: f32 = 40.0;
pub(crate) const CONTROL_WIDTH: f32 = 44.0;
pub(crate) const CONTROL_COUNT: f32 = 3.0;
pub(crate) const CONTROLS_WIDTH: f32 = CONTROL_WIDTH * CONTROL_COUNT;

const TITLE_BAR_BACKGROUND: Color = Color::rgba(255, 255, 255, 245);

const TITLE_BAR_BORDER: Color = Color::rgba(0, 0, 0, 26);

const TITLE_COLOR: Color = Color::from_rgb_hex(0x171717);

const CONTROL_COLOR: Color = Color::from_rgb_hex(0x202020);

const CONTROL_HOVER_BACKGROUND: Color = Color::rgba(0, 0, 0, 17);

const CLOSE_HOVER_BACKGROUND: Color = Color::from_rgb_hex(0xE81123);

pub(crate) struct WindowDecoration {
    title: String,
    interaction: WindowInteraction,
}

impl WindowDecoration {
    pub(crate) fn new(title: impl Into<String>, interaction: WindowInteraction) -> Self {
        Self {
            title: title.into(),
            interaction,
        }
    }

    fn control_bounds(bounds: Rect, index: usize) -> Rect {
        Rect::new(
            bounds.origin.x + bounds.size.width - CONTROLS_WIDTH + CONTROL_WIDTH * index as f32,
            bounds.origin.y,
            CONTROL_WIDTH,
            TITLE_BAR_HEIGHT,
        )
    }

    fn paint_control(
        bounds: Rect,
        hovered: bool,
        pressed: bool,
        close: bool,
        icon: ControlIcon,
        context: &mut PaintContext<'_>,
    ) {
        let background = if close && pressed {
            Color::from_rgb_hex(0xC50F1F)
        } else if close && hovered {
            CLOSE_HOVER_BACKGROUND
        } else if pressed {
            Color::rgba(0, 0, 0, 26)
        } else if hovered {
            CONTROL_HOVER_BACKGROUND
        } else {
            Color::TRANSPARENT
        };

        let foreground = if close && (hovered || pressed) {
            Color::WHITE
        } else {
            CONTROL_COLOR
        };

        Rectangle::new()
            .color(RectangleColor::Custom(background))
            .radius(CornerRadius::None)
            .paint(bounds, context);

        let icon_bounds = Rect::new(
            bounds.origin.x + (bounds.size.width - 14.0) / 2.0,
            bounds.origin.y + (bounds.size.height - 14.0) / 2.0,
            14.0,
            14.0,
        );

        control_icon(icon, foreground).paint(icon_bounds, context);
    }
}

impl View for WindowDecoration {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(constraints.maximum.width, TITLE_BAR_HEIGHT))
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Rectangle::new()
            .color(RectangleColor::Custom(TITLE_BAR_BACKGROUND))
            .radius(CornerRadius::None)
            .paint(bounds, context);

        let border_bounds = Rect::new(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height - 1.0,
            bounds.size.width,
            1.0,
        );

        Rectangle::new()
            .color(RectangleColor::Custom(TITLE_BAR_BORDER))
            .paint(border_bounds, context);

        Text::new(self.title.clone())
            .font_size(12.0)
            .line_height(18.0)
            .alignment(TextAlignment::Center)
            .color(TITLE_COLOR)
            .paint(
                Rect::new(
                    bounds.origin.x,
                    bounds.origin.y + (TITLE_BAR_HEIGHT - 18.0) / 2.0,
                    bounds.size.width,
                    18.0,
                ),
                context,
            );

        let minimize_hovered = self.interaction.hovered == Some(WindowControl::Minimize);

        let maximize_hovered = self.interaction.hovered == Some(WindowControl::Maximize);

        let close_hovered = self.interaction.hovered == Some(WindowControl::Close);

        Self::paint_control(
            Self::control_bounds(bounds, 0),
            minimize_hovered,
            minimize_hovered && self.interaction.pressed == Some(WindowControl::Minimize),
            false,
            ControlIcon::Minimize,
            context,
        );

        Self::paint_control(
            Self::control_bounds(bounds, 1),
            maximize_hovered,
            maximize_hovered && self.interaction.pressed == Some(WindowControl::Maximize),
            false,
            ControlIcon::Maximize,
            context,
        );

        Self::paint_control(
            Self::control_bounds(bounds, 2),
            close_hovered,
            close_hovered && self.interaction.pressed == Some(WindowControl::Close),
            true,
            ControlIcon::Close,
            context,
        );
    }
}

#[derive(Clone, Copy)]
enum ControlIcon {
    Minimize,
    Maximize,
    Close,
}

fn control_icon(icon: ControlIcon, color: Color) -> Svg {
    static CLOSE: OnceLock<SvgData> = OnceLock::new();

    static MAXIMIZE: OnceLock<SvgData> = OnceLock::new();

    static MINIMIZE: OnceLock<SvgData> = OnceLock::new();

    let data = match icon {
        ControlIcon::Close => CLOSE.get_or_init(|| {
            SvgData::decode(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/close.svg",
            )))
            .expect("resources/close.svg is invalid")
        }),

        ControlIcon::Maximize => MAXIMIZE.get_or_init(|| {
            SvgData::decode(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/maximize.svg",
            )))
            .expect("resources/maximize.svg is invalid")
        }),

        ControlIcon::Minimize => MINIMIZE.get_or_init(|| {
            SvgData::decode(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/minimize.svg",
            )))
            .expect("resources/minimize.svg is invalid")
        }),
    };

    Svg::new(data.clone()).tint(color)
}

pub(crate) fn title_bar_bounds(frame: Rect) -> Rect {
    Rect::new(
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        TITLE_BAR_HEIGHT,
    )
}

pub(crate) fn minimize_bounds(frame: Rect) -> Rect {
    WindowDecoration::control_bounds(frame, 0)
}

pub(crate) fn maximize_bounds(frame: Rect) -> Rect {
    WindowDecoration::control_bounds(frame, 1)
}

pub(crate) fn close_bounds(frame: Rect) -> Rect {
    WindowDecoration::control_bounds(frame, 2)
}
