use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

use crate::window::{DesktopWindow, DesktopWindows, WindowControl, WindowDrag};

use super::{window, window_decoration};

const DESKTOP_TOP_INSET: f32 = 40.0;

pub(crate) struct WindowLayer<C> {
    content: C,

    windows: State<DesktopWindows>,

    drag: State<Option<WindowDrag>>,
}

impl<C> WindowLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,

        windows: State<DesktopWindows>,

        drag: State<Option<WindowDrag>>,
    ) -> Self {
        Self {
            content,
            windows,
            drag,
        }
    }

    fn topmost_window_at(desktop: &DesktopWindows, position: Point) -> Option<DesktopWindow> {
        desktop
            .windows
            .iter()
            .rev()
            .find(|desktop_window| {
                !desktop_window.minimized && window::contains(desktop_window.frame, position)
            })
            .cloned()
    }

    fn move_dragged_window(&self, bounds: Rect, drag: WindowDrag, pointer: Point) {
        let delta_x = pointer.x - drag.pointer_origin.x;

        let delta_y = pointer.y - drag.pointer_origin.y;

        self.windows.update(|desktop| {
            let Some(window) = desktop
                .windows
                .iter_mut()
                .find(|window| window.id == drag.window)
            else {
                return;
            };

            let minimum_x = bounds.origin.x;

            let minimum_y = bounds.origin.y + DESKTOP_TOP_INSET;

            let maximum_x =
                (bounds.origin.x + bounds.size.width - window.frame.size.width).max(minimum_x);

            let maximum_y =
                (bounds.origin.y + bounds.size.height - window.frame.size.height).max(minimum_y);

            let x = (drag.window_origin.x + delta_x).clamp(minimum_x, maximum_x);

            let y = (drag.window_origin.y + delta_y).clamp(minimum_y, maximum_y);

            window.frame.origin = Point::new(x, y);
        });
    }

    fn control_at(frame: Rect, position: Point) -> Option<WindowControl> {
        if window_decoration::close_bounds(frame).contains(position) {
            return Some(WindowControl::Close);
        }

        if window_decoration::maximize_bounds(frame).contains(position) {
            return Some(WindowControl::Maximize);
        }

        if window_decoration::minimize_bounds(frame).contains(position) {
            return Some(WindowControl::Minimize);
        }

        None
    }

    fn update_hovered_control(&self, position: Point) -> bool {
        let desktop = self.windows.get();

        let target = Self::topmost_window_at(&desktop, position).and_then(|window| {
            Self::control_at(window.frame, position).map(|control| (window.id, control))
        });

        let changed = desktop.windows.iter().any(|window| {
            let expected = match target {
                Some((id, control)) if id == window.id => Some(control),

                _ => None,
            };

            window.interaction.hovered != expected
        });

        drop(desktop);

        if changed {
            self.windows.update(|desktop| {
                desktop.set_hovered_control(target);
            });
        }

        changed
    }

    fn clear_interactions(&self) -> bool {
        let desktop = self.windows.get();

        let changed = desktop.windows.iter().any(|window| {
            window.interaction.hovered.is_some() || window.interaction.pressed.is_some()
        });

        drop(desktop);

        if changed {
            self.windows.update(|desktop| {
                desktop.clear_interactions();
            });
        }

        changed
    }

    fn desktop_work_area(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + DESKTOP_TOP_INSET,
            bounds.size.width,
            (bounds.size.height - DESKTOP_TOP_INSET).max(0.0),
        )
    }
}

impl<C> View for WindowLayer<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);

        let desktop = self.windows.get();

        for desktop_window in &desktop.windows {
            if desktop_window.minimized {
                continue;
            }

            let focused = desktop.focused == Some(desktop_window.id);

            window::view(desktop_window, focused).paint(desktop_window.frame, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(drag) = self.drag.get() {
            return match event {
                ViewEvent::PointerMoved { position } => {
                    self.move_dragged_window(bounds, drag, *position);

                    context.request_redraw();

                    EventResult::Consumed
                }

                ViewEvent::PointerReleased {
                    button: PointerButton::Primary,
                    ..
                }
                | ViewEvent::PointerLeft
                | ViewEvent::FocusChanged { focused: false } => {
                    self.drag.set(None);

                    context.request_redraw();

                    EventResult::Consumed
                }

                _ => EventResult::Consumed,
            };
        }

        match event {
            ViewEvent::PointerMoved { position } => {
                if self.update_hovered_control(*position) {
                    context.request_redraw();
                }

                let desktop = self.windows.get();

                if Self::topmost_window_at(&desktop, *position).is_some() {
                    EventResult::Consumed
                } else {
                    drop(desktop);

                    self.content.handle_event(bounds, event, context)
                }
            }

            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                let hit_window = {
                    let desktop = self.windows.get();

                    Self::topmost_window_at(&desktop, *position)
                };

                let Some(hit_window) = hit_window else {
                    let is_desktop_area = position.y >= bounds.origin.y + DESKTOP_TOP_INSET;

                    if is_desktop_area {
                        let desktop = self.windows.get();

                        let changed = desktop.focused.is_some()
                            || desktop.windows.iter().any(|window| {
                                window.interaction.hovered.is_some()
                                    || window.interaction.pressed.is_some()
                            });

                        drop(desktop);

                        if changed {
                            self.windows.update(|desktop| {
                                desktop.focused = None;

                                desktop.clear_interactions();
                            });

                            context.request_redraw();
                        }
                    }

                    return self.content.handle_event(bounds, event, context);
                };

                let control = Self::control_at(hit_window.frame, *position);

                self.windows.update(|desktop| {
                    desktop.focus(hit_window.id);

                    desktop.clear_pressed_controls();

                    if let Some(control) = control {
                        desktop.press_control(hit_window.id, control);
                    }
                });

                let in_title_bar =
                    window_decoration::title_bar_bounds(hit_window.frame).contains(*position);

                if control.is_none() && in_title_bar && hit_window.restore_frame.is_none() {
                    self.drag.set(Some(WindowDrag {
                        window: hit_window.id,

                        pointer_origin: *position,

                        window_origin: hit_window.frame.origin,
                    }));
                }

                context.request_redraw();

                EventResult::Consumed
            }

            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let pressed = {
                    let desktop = self.windows.get();

                    desktop.windows.iter().find_map(|window| {
                        window
                            .interaction
                            .pressed
                            .map(|control| (window.id, control, window.frame))
                    })
                };

                if let Some((window_id, pressed_control, frame)) = pressed {
                    let released_control = Self::control_at(frame, *position);

                    let activate = released_control == Some(pressed_control);

                    let work_area = Self::desktop_work_area(bounds);

                    self.windows.update(|desktop| {
                        desktop.clear_pressed_controls();

                        if !activate {
                            return;
                        }

                        match pressed_control {
                            WindowControl::Minimize => {
                                desktop.minimize(window_id);
                            }

                            WindowControl::Maximize => {
                                desktop.toggle_maximize(window_id, work_area);
                            }

                            WindowControl::Close => {
                                desktop.close(window_id);
                            }
                        }
                    });

                    self.update_hovered_control(*position);

                    context.request_redraw();

                    return EventResult::Consumed;
                }

                let desktop = self.windows.get();

                if Self::topmost_window_at(&desktop, *position).is_some() {
                    EventResult::Consumed
                } else {
                    drop(desktop);

                    self.content.handle_event(bounds, event, context)
                }
            }

            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                if self.clear_interactions() {
                    context.request_redraw();
                }

                self.content.handle_event(bounds, event, context)
            }

            _ => {
                if let Some(position) = event.position() {
                    let desktop = self.windows.get();

                    if Self::topmost_window_at(&desktop, position).is_some() {
                        return EventResult::Consumed;
                    }
                }

                self.content.handle_event(bounds, event, context)
            }
        }
    }
}
