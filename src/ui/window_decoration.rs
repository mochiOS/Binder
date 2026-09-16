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
            Theme::current().shell.close_pressed
        } else if close && hovered {
            Theme::current().shell.close_hover
        } else if pressed {
            Theme::current().shell.title_bar_border
        } else if hovered {
            Theme::current().shell.control_hover
        } else {
            Color::TRANSPARENT
        };

        let foreground = if close && (hovered || pressed) {
            Theme::current().shell.inverse_text
        } else {
            Theme::current().shell.control
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
            .color(RectangleColor::Custom(
                Theme::current().shell.title_bar_background,
            ))
            .radius(CornerRadius::None)
            .paint(bounds, context);

        let border_bounds = Rect::new(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height - 1.0,
            bounds.size.width,
            1.0,
        );

        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.title_bar_border,
            ))
            .paint(border_bounds, context);

        let title_line_height = context.typography.caption.line_height;
        Text::styled(self.title.clone(), TextRole::Caption)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    bounds.origin.x,
                    bounds.origin.y + (TITLE_BAR_HEIGHT - title_line_height) / 2.0,
                    bounds.size.width,
                    title_line_height,
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
