use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

use crate::window::{DesktopWindow, DesktopWindows, WindowDrag};

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

        if let ViewEvent::PointerPressed {
            position,
            button: PointerButton::Primary,
        } = event
        {
            let hit_window = {
                let desktop = self.windows.get();

                topmost_window_at(&desktop, *position)
            };

            let Some(hit_window) = hit_window else {
                let is_desktop_area = position.y >= bounds.origin.y + DESKTOP_TOP_INSET;

                if is_desktop_area {
                    let has_focused_window = self.windows.get().focused.is_some();

                    if has_focused_window {
                        self.windows.update(|desktop| {
                            desktop.focused = None;
                        });

                        context.request_redraw();
                    }
                }

                return self.content.handle_event(bounds, event, context);
            };

            if window_decoration::close_bounds(hit_window.frame).contains(*position) {
                self.windows.update(|desktop| {
                    desktop.close(hit_window.id);
                });

                self.drag.set(None);
                context.request_redraw();

                return EventResult::Consumed;
            }

            self.windows.update(|desktop| {
                desktop.focus(hit_window.id);
            });

            if window_decoration::title_bar_bounds(hit_window.frame).contains(*position) {
                self.drag.set(Some(WindowDrag {
                    window: hit_window.id,

                    pointer_origin: *position,

                    window_origin: hit_window.frame.origin,
                }));
            }

            context.request_redraw();

            return EventResult::Consumed;
        }

        if let Some(position) = event.position() {
            let desktop = self.windows.get();

            if Self::topmost_window_at(&desktop, position).is_some() {
                return EventResult::Consumed;
            }
        }

        self.content.handle_event(bounds, event, context)
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
