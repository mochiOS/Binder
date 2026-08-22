use super::{about, launch_failure, test, window_decoration};
use crate::apps;
use crate::window::DesktopWindow;
use viewkit::{
    draw_command::DrawCommand,
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) const WINDOW_CORNER_RADIUS: f32 = 14.0;
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
            .color(RectangleColor::Custom(
                Theme::current().shell.window_background,
            ))
            .paint(bounds, context);

        self.decoration
            .paint(Self::title_bar_bounds(bounds), context);

        self.content.paint(Self::content_bounds(bounds), context);

        context.display_list.push(DrawCommand::PopClip);

        if self.focused {
            Rectangle::new()
                .color(RectangleColor::Custom(Color::TRANSPARENT))
                .radius(CornerRadius::Custom(WINDOW_CORNER_RADIUS))
                .border(BorderStyle::custom(
                    Theme::current().shell.window_border,
                    1.0,
                ))
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

pub(crate) fn view(
    window: &DesktopWindow,
    focused: bool,
    test_state: Option<test::TestWindowState>,
    launch_failure_state: Option<launch_failure::LaunchFailureWindowState>,
    windows: State<crate::window::DesktopWindows>,
) -> impl View + 'static {
    let content: Box<dyn View + 'static> = match window.renderer.as_str() {
        apps::ABOUT_ENTRY => Box::new(about::view()),
        apps::LAUNCH_FAILURE_ENTRY => Box::new(launch_failure::view(
            launch_failure_state.unwrap_or_default(),
            window.id,
            windows,
        )),
        apps::TEST_ENTRY => Box::new(test::view(test_state.unwrap_or_default())),
        _ => Box::new(remote_placeholder_view()),
    };

    WindowView {
        decoration: window_decoration::WindowDecoration::new(
            window.title.clone(),
            window.interaction,
        ),

        content,

        shadow: if focused {
            ShadowStyle::Custom(Theme::current().shell.active_window_shadow)
        } else {
            ShadowStyle::Custom(Theme::current().shell.inactive_window_shadow)
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

fn remote_placeholder_view() -> impl View + 'static {
    VStack::new()
        .alignment(StackAlignment::Center)
        .distribution(StackDistribution::Center)
        .gap(StackGap::Small)
        .child(
            Text::new("RemoteSurface")
                .font_size(24.0)
                .line_height(32.0)
                .weight(750)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.control),
        )
        .child(
            Text::new("No compositor surface attached")
                .font_size(13.0)
                .line_height(18.0)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.tertiary_text),
        )
}
