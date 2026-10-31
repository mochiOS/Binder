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
            context.theme.shell.close_pressed
        } else if close && hovered {
            context.theme.shell.close_hover
        } else if pressed {
            context.theme.shell.title_bar_border
        } else if hovered {
            context.theme.shell.control_hover
        } else {
            Color::TRANSPARENT
        };

        let foreground = if close && (hovered || pressed) {
            context.theme.shell.inverse_text
        } else {
            context.theme.shell.control
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
                context.theme.shell.title_bar_background,
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
            .color(RectangleColor::Custom(context.theme.shell.title_bar_border))
            .paint(border_bounds, context);

        let title = Text::styled(self.title.clone(), TextRole::Label)
            .weight(context.typography.label.weight.0)
            .center_visible_ink(true)
            .color(context.theme.shell.primary_text);
        let measured = title.measure(
            Constraints::loose(Size::new(f32::INFINITY, TITLE_BAR_HEIGHT)),
            &mut MeasureContext {
                theme: context.theme,
                typography: context.typography,
                text_measurer: context.text_measurer,
            },
        );
        let title_bounds = title_bounds(bounds, measured.width, context.theme.spacing.medium);
        title
            .alignment(if title_bounds.1 {
                TextAlignment::Center
            } else {
                TextAlignment::Start
            })
            .paint(title_bounds.0, context);

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

// Centre when the title fits symmetrically; narrow utility windows retain a
// leading title instead of losing it to two fixed 144px reserved regions.
fn title_bounds(bounds: Rect, width: f32, gap: f32) -> (Rect, bool) {
    let symmetric_inset = CONTROLS_WIDTH + gap;
    if width <= (bounds.size.width - symmetric_inset * 2.0).max(0.0) {
        (
            Rect::new(
                bounds.origin.x + symmetric_inset,
                bounds.origin.y,
                (bounds.size.width - symmetric_inset * 2.0).max(0.0),
                bounds.size.height,
            ),
            true,
        )
    } else {
        (
            Rect::new(
                bounds.origin.x + gap,
                bounds.origin.y,
                (bounds.size.width - CONTROLS_WIDTH - gap * 2.0).max(0.0),
                bounds.size.height,
            ),
            false,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use viewkit::{
        draw_command::{DisplayList, DrawCommand},
        typography::TextMeasurer,
    };
    #[test]
    fn narrow_utility_titles_remain_visible_and_never_overlap_controls() {
        for width in [240., 280., 360., 800.] {
            let bounds = Rect::new(10., 20., width, TITLE_BAR_HEIGHT);
            let (title, _) = title_bounds(bounds, 80., 12.);
            assert!(title.size.width > 0.);
            assert!(title.origin.x + title.size.width <= bounds.origin.x + width - CONTROLS_WIDTH);
        }
        let (title, centered) = title_bounds(Rect::new(0., 0., 800., 40.), 80., 12.);
        assert!(centered);
        assert_eq!(title.origin.x + title.size.width * 0.5, 400.);
    }
    #[test]
    fn decoration_uses_context_material_and_native_label_weight() {
        for theme in [Theme::LIGHT, Theme::DARK] {
            let mut list = DisplayList::new();
            let mut text = TextMeasurer::new();
            WindowDecoration::new("Calculator", WindowInteraction::default()).paint(
                Rect::new(0., 0., 280., 40.),
                &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut text),
            );
            assert!(list.commands().iter().any(|cmd| matches!(cmd,DrawCommand::FillRect {color,..} if *color==theme.shell.title_bar_background)));
            assert!(list.commands().iter().any(|cmd| matches!(cmd,DrawCommand::DrawText {command} if command.text=="Calculator" && command.bounds.size.width>0. && command.weight==theme.typography.label.weight.0 && command.color==theme.shell.primary_text)));
        }
    }
}
