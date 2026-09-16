use std::cell::Cell;
use std::rc::Rc;

use crate::platform::{AppInfo, PlatformError};
use crate::window::{DesktopWindows, WindowId};
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

#[derive(Clone)]
pub(crate) struct LaunchFailureWindowState {
    app_name: String,
    message: String,
    guidance: String,
    detail: String,
    button_hovered: Rc<Cell<bool>>,
    button_pressed: Rc<Cell<bool>>,
}

impl LaunchFailureWindowState {
    pub(crate) fn new(app: &AppInfo, error: PlatformError) -> Self {
        let (message, guidance, detail) = match error {
            PlatformError::ProcessLaunchRejected { errno: 1 } | PlatformError::PermissionDenied => {
                (
                    "mochiOS blocked this app.",
                    "Its requested capabilities are not permitted by system policy.",
                    "Capability policy rejected the launch (error 1).".to_string(),
                )
            }
            PlatformError::ProcessLaunchRejected { errno: 2 } => (
                "mochiOS could not find a required file.",
                "Reinstall the app or check its package contents.",
                "Capability policy rejected the launch (error 2).".to_string(),
            ),
            PlatformError::ProcessLaunchRejected { errno } => (
                "mochiOS blocked this app.",
                "Check the app's capabilities or ask an administrator.",
                format!("Capability policy rejected the launch (error {errno})."),
            ),
            _ => (
                "The app could not be started.",
                "Check its installation and permissions, then try again.",
                "Application launch failed.".to_string(),
            ),
        };

        Self {
            app_name: app.name.clone(),
            message: message.to_string(),
            guidance: guidance.to_string(),
            detail,
            button_hovered: Rc::new(Cell::new(false)),
            button_pressed: Rc::new(Cell::new(false)),
        }
    }
}

impl Default for LaunchFailureWindowState {
    fn default() -> Self {
        Self {
            app_name: String::from("Application"),
            message: String::from("The app could not be started."),
            guidance: String::from("Check its installation and permissions, then try again."),
            detail: String::from("Application launch failed."),
            button_hovered: Rc::new(Cell::new(false)),
            button_pressed: Rc::new(Cell::new(false)),
        }
    }
}

pub(crate) fn view(
    state: LaunchFailureWindowState,
    window_id: WindowId,
    windows: State<DesktopWindows>,
) -> impl View + 'static {
    LaunchFailureView {
        state,
        window_id,
        windows,
    }
}

struct LaunchFailureView {
    state: LaunchFailureWindowState,
    window_id: WindowId,
    windows: State<DesktopWindows>,
}

impl LaunchFailureView {
    fn button_bounds(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x + bounds.size.width - 104.0,
            bounds.origin.y + bounds.size.height - 48.0,
            80.0,
            32.0,
        )
    }

    fn close(&self, bounds: Rect, context: &mut EventContext<'_>) {
        self.state.button_hovered.set(false);
        self.state.button_pressed.set(false);
        self.windows.update(|desktop| desktop.close(self.window_id));
        context.request_redraw_in(bounds.expanded(24.0));
    }
}

impl View for LaunchFailureView {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(420.0, 210.0))
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let mut dialog = AccessibilityNode::new(AccessibilityRole::Dialog, bounds);
        dialog.label = Some(format!("{} could not be opened", self.state.app_name));
        context.record_accessibility(dialog);

        let icon = Rect::new(bounds.origin.x + 24.0, bounds.origin.y + 28.0, 42.0, 42.0);
        Ellipse::new()
            .color(EllipseColor::Custom(Theme::current().shell.alert))
            .paint(icon, context);
        Text::new("!")
            .accessibility_hidden(true)
            .font_size(26.0)
            .line_height(42.0)
            .weight(750)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.inverse_text)
            .paint(icon, context);

        let text_x = bounds.origin.x + 82.0;
        let text_width = (bounds.size.width - 106.0).max(0.0);
        Text::styled(
            format!("{} could not be opened", self.state.app_name),
            TextRole::TitleSmall,
        )
            .color(Theme::current().shell.control)
            .paint(
                Rect::new(text_x, bounds.origin.y + 24.0, text_width, 24.0),
                context,
            );
        Text::styled(self.state.message.clone(), TextRole::Label)
            .color(Theme::current().shell.secondary_text)
            .paint(
                Rect::new(text_x, bounds.origin.y + 55.0, text_width, 20.0),
                context,
            );
        Text::styled(self.state.guidance.clone(), TextRole::Caption)
            .color(Theme::current().shell.secondary_text)
            .paint(
                Rect::new(text_x, bounds.origin.y + 78.0, text_width, 18.0),
                context,
            );
        Text::styled(self.state.detail.clone(), TextRole::Caption)
            .color(Theme::current().shell.tertiary_text)
            .paint(
                Rect::new(text_x, bounds.origin.y + 104.0, text_width, 17.0),
                context,
            );

        let button = Self::button_bounds(bounds);
        let mut button_node = AccessibilityNode::new(AccessibilityRole::Button, button);
        button_node.label = Some(String::from("OK"));
        button_node.focusable = true;
        context.record_accessibility(button_node);
        let button_color = if self.state.button_pressed.get() {
            Theme::current().shell.action_pressed
        } else if self.state.button_hovered.get() {
            Theme::current().shell.action_hover
        } else {
            Theme::current().shell.action
        };
        Rectangle::new()
            .color(RectangleColor::Custom(button_color))
            .radius(CornerRadius::Custom(7.0))
            .paint(button, context);
        let button_label_style = Theme::current().typography.style(TextRole::Label);
        Text::styled("OK", TextRole::Label)
            .accessibility_hidden(true)
            .weight(650)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.inverse_text)
            .paint(
                Rect::new(
                    button.origin.x,
                    button.origin.y
                        + (button.size.height - button_label_style.line_height) / 2.0,
                    button.size.width,
                    button_label_style.line_height,
                ),
                context,
            );
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let button = Self::button_bounds(bounds);
        match event {
            ViewEvent::KeyPressed { key, .. } if matches!(key, Key::Enter | Key::Escape) => {
                self.close(bounds, context);
                EventResult::Consumed
            }
            ViewEvent::PointerMoved { position } => {
                let hovered = button.contains(*position);
                if self.state.button_hovered.replace(hovered) != hovered {
                    context.request_redraw_in(button.expanded(4.0));
                }
                EventResult::Consumed
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } if button.contains(*position) => {
                self.state.button_pressed.set(true);
                context.request_redraw_in(button.expanded(4.0));
                EventResult::Consumed
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let activate =
                    self.state.button_pressed.replace(false) && button.contains(*position);
                if activate {
                    self.close(bounds, context);
                } else {
                    context.request_redraw_in(button.expanded(4.0));
                }
                EventResult::Consumed
            }
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                let changed = self.state.button_hovered.replace(false)
                    || self.state.button_pressed.replace(false);
                if changed {
                    context.request_redraw_in(button.expanded(4.0));
                }
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }
}
