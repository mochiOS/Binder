use super::{about, control_center_editor, launch_failure, test, window_decoration};
use crate::apps;
use crate::control_center_preferences::ControlCenterPreferences;
use crate::platform::AppInfo;
use crate::window::DesktopWindow;
use std::cell::RefCell;
use std::rc::Rc;
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
    radius: f32,
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
            .radius(CornerRadius::Custom(self.radius))
            .shadow(self.shadow)
            .paint(bounds, context);

        context.display_list.push(DrawCommand::PushRoundedClip {
            rect: bounds,
            radius: self.radius,
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
                .radius(CornerRadius::Custom(self.radius))
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
    control_center_preferences: Rc<RefCell<ControlCenterPreferences>>,
    control_center_editor_interaction: Rc<
        RefCell<control_center_editor::ControlCenterEditorInteraction>,
    >,
    apps: State<Vec<AppInfo>>,
) -> impl View + 'static {
    let content: Box<dyn View + 'static> = match window.renderer.as_str() {
        apps::ABOUT_ENTRY => Box::new(about::view()),
        apps::LAUNCH_FAILURE_ENTRY => Box::new(launch_failure::view(
            launch_failure_state.unwrap_or_default(),
            window.id,
            windows,
        )),
        apps::CONTROL_CENTER_EDITOR_ENTRY => Box::new(control_center_editor::view(
            control_center_preferences,
            control_center_editor_interaction,
            apps,
        )),
        apps::TEST_ENTRY => Box::new(test::view(test_state.unwrap_or_default())),
        _ => remote_content_view(&window.renderer),
    };

    WindowView {
        decoration: window_decoration::WindowDecoration::new(
            window.title.clone(),
            window.interaction,
        ),

        content,

        shadow: if window.restore_frame.is_some() {
            ShadowStyle::None
        } else if focused {
            ShadowStyle::Custom(Theme::current().shell.active_window_shadow)
        } else {
            ShadowStyle::Custom(Theme::current().shell.inactive_window_shadow)
        },
        focused,
        radius: corner_radius(window),
    }
}

pub(crate) fn corner_radius(window: &DesktopWindow) -> f32 {
    if window.restore_frame.is_some() {
        0.0
    } else {
        WINDOW_CORNER_RADIUS
    }
}

pub(crate) fn contains(window: &DesktopWindow, point: Point) -> bool {
    window.frame.contains_rounded(corner_radius(window), point)
}

fn remote_content_view(renderer: &str) -> Box<dyn View> {
    #[cfg(target_os = "linux")]
    if let Some(surface) = super::host_surface::HostSurface::for_renderer(renderer) {
        return Box::new(surface);
    }
    let _ = renderer;
    Box::new(remote_placeholder_view())
}

fn remote_placeholder_view() -> impl View + 'static {
    VStack::new()
        .alignment(StackAlignment::Center)
        .distribution(StackDistribution::Center)
        .gap(StackGap::Small)
        .child(
            Text::styled("RemoteSurface", TextRole::TitleMedium)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.control),
        )
        .child(
            Text::styled("No compositor surface attached", TextRole::Label)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.tertiary_text),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewkit::{draw_command::DisplayList, typography::TextMeasurer};
    #[test]
    fn maximizing_and_restoring_match_paint_clip_and_hit_test() {
        let mut desktop = crate::window::DesktopWindows::default();
        let (id, _) =
            desktop.open_test(crate::platform::ProcessId(1), "Test".into(), 640, 480, true);
        let display = Rect::new(0.0, 0.0, 1280.0, 800.0);
        for maximized in [false, true, false] {
            if desktop.windows[0].restore_frame.is_some() != maximized {
                desktop.toggle_maximize(id, display);
            }
            let window = &desktop.windows[0];
            let radius = corner_radius(window);
            assert_eq!(radius, if maximized { 0.0 } else { WINDOW_CORNER_RADIUS });
            assert_eq!(
                contains(
                    window,
                    Point::new(window.frame.origin.x + 0.01, window.frame.origin.y + 0.01)
                ),
                maximized
            );
            let view = WindowView {
                decoration: window_decoration::WindowDecoration::new("Test", window.interaction),
                content: Box::new(Rectangle::new()),
                shadow: ShadowStyle::None,
                focused: true,
                radius,
            };
            let theme = Theme::current();
            let mut measurer = TextMeasurer::new();
            let mut list = DisplayList::new();
            view.paint(
                window.frame,
                &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
            );
            assert!(list.commands().iter().any(|command| matches!(command, DrawCommand::PushRoundedClip { rect, radius: actual } if *rect == window.frame && *actual == radius)));
        }
    }
}
