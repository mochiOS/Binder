use super::{about, window_decoration};
use crate::window::{DesktopWindow, WindowContent};
use viewkit::theme::{Shadow, ShadowSet};
use viewkit::{
    draw_command::DrawCommand,
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) const WINDOW_CORNER_RADIUS: f32 = 14.0;
const WINDOW_BACKGROUND: Color = Color::rgba(250, 250, 250, 235);
const WINDOW_BORDER: Color = Color::rgba(0, 0, 0, 31);

const INACTIVE_WINDOW_SHADOW: ShadowSet =
    ShadowSet::single(Shadow::new(Color::rgba(0, 0, 0, 8), 0.0, 2.0, 5.0, 1.0));

const ACTIVE_WINDOW_SHADOW: ShadowSet = ShadowSet::double(
    Shadow::new(Color::rgba(0, 0, 0, 20), 0.0, 2.0, 5.0, 0.0),
    Shadow::new(Color::rgba(0, 0, 0, 28), 0.0, 5.0, 14.0, 0.0),
);

struct WindowView {
    decoration: window_decoration::WindowDecoration,
    content: Box<dyn View + 'static>,
    shadow: ShadowStyle,
    focused: bool,
}

impl WindowView {
    fn title_bar_bounds(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            window_decoration::TITLE_BAR_HEIGHT,
        )
    }

    fn content_bounds(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + window_decoration::TITLE_BAR_HEIGHT,
            bounds.size.width,
            (bounds.size.height - window_decoration::TITLE_BAR_HEIGHT).max(0.0),
        )
    }
}

impl View for WindowView {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        let maximum_content_height =
            (constraints.maximum.height - window_decoration::TITLE_BAR_HEIGHT).max(0.0);

        let content_size = self.content.measure(
            Constraints::loose(Size::new(constraints.maximum.width, maximum_content_height)),
            context,
        );

        constraints.constrain(Size::new(
            content_size.width,
            content_size.height + window_decoration::TITLE_BAR_HEIGHT,
        ))
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        if bounds.is_empty() {
            return;
        }

        Rectangle::new()
            .color(RectangleColor::Custom(Color::TRANSPARENT))
            .radius(CornerRadius::Custom(WINDOW_CORNER_RADIUS))
            .shadow(self.shadow)
            .paint(bounds, context);

        context.display_list.push(DrawCommand::PushRoundedClip {
            rect: bounds,
            radius: WINDOW_CORNER_RADIUS,
        });

        Rectangle::new()
            .color(RectangleColor::Custom(WINDOW_BACKGROUND))
            .paint(bounds, context);

        self.decoration
            .paint(Self::title_bar_bounds(bounds), context);

        self.content.paint(Self::content_bounds(bounds), context);

        context.display_list.push(DrawCommand::PopClip);

        if self.focused {
            Rectangle::new()
                .color(RectangleColor::Custom(Color::TRANSPARENT))
                .radius(CornerRadius::Custom(WINDOW_CORNER_RADIUS))
                .border(BorderStyle::custom(WINDOW_BORDER, 1.0))
                .paint(bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,

        context: &mut EventContext<'_>,
    ) -> EventResult {
        let decoration_result =
            self.decoration
                .handle_event(Self::title_bar_bounds(bounds), event, context);

        let content_result =
            self.content
                .handle_event(Self::content_bounds(bounds), event, context);

        decoration_result.merge(content_result)
    }
}

pub(crate) fn view(window: &DesktopWindow, focused: bool) -> impl View + 'static {
    let content: Box<dyn View + 'static> = match window.content {
        WindowContent::About => Box::new(about::view()),
    };

    WindowView {
        decoration: window_decoration::WindowDecoration::new(
            window.title.clone(),
            window.interaction,
        ),

        content,

        shadow: if focused {
            ShadowStyle::Custom(ACTIVE_WINDOW_SHADOW)
        } else {
            ShadowStyle::Custom(INACTIVE_WINDOW_SHADOW)
        },
        focused,
    }
}

pub(crate) fn contains(bounds: Rect, point: Point) -> bool {
    if !bounds.contains(point) {
        return false;
    }

    let radius = WINDOW_CORNER_RADIUS
        .max(0.0)
        .min(bounds.size.width.min(bounds.size.height) / 2.0);

    if radius == 0.0 {
        return true;
    }

    let left = bounds.origin.x;

    let top = bounds.origin.y;

    let right = left + bounds.size.width;

    let bottom = top + bounds.size.height;

    if point.x >= left + radius && point.x < right - radius {
        return true;
    }

    if point.y >= top + radius && point.y < bottom - radius {
        return true;
    }

    let center_x = if point.x < left + radius {
        left + radius
    } else {
        right - radius
    };

    let center_y = if point.y < top + radius {
        top + radius
    } else {
        bottom - radius
    };

    let delta_x = point.x - center_x;

    let delta_y = point.y - center_y;

    delta_x * delta_x + delta_y * delta_y <= radius * radius
}
