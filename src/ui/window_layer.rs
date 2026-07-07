use crate::desktop::{ResizeEdge, WindowResize};
use crate::window::{DesktopWindow, DesktopWindows, WindowControl, WindowDrag};
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

use super::{window, window_decoration};

const DESKTOP_TOP_INSET: f32 = 40.0;
const RESIZE_HANDLE_SIZE: f32 = 10.0;
const MINIMUM_WINDOW_WIDTH: f32 = 280.0;
const MINIMUM_WINDOW_HEIGHT: f32 = 180.0;

pub(crate) struct WindowLayer<C> {
    content: C,
    windows: State<DesktopWindows>,
    drag: State<Option<WindowDrag>>,
    resize: State<Option<WindowResize>>,
}

impl<C> WindowLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        windows: State<DesktopWindows>,
        drag: State<Option<WindowDrag>>,
        resize: State<Option<WindowResize>>,
    ) -> Self {
        Self {
            content,
            windows,
            drag,
            resize,
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

    fn resize_edge_at(frame: Rect, position: Point) -> Option<ResizeEdge> {
        let left = frame.origin.x;
        let top = frame.origin.y;

        let right = frame.origin.x + frame.size.width;

        let bottom = frame.origin.y + frame.size.height;

        let inside_horizontal = position.x >= left && position.x <= right;

        let inside_vertical = position.y >= top && position.y <= bottom;

        if !inside_horizontal || !inside_vertical {
            return None;
        }

        let near_left = position.x <= left + RESIZE_HANDLE_SIZE;

        let near_right = position.x >= right - RESIZE_HANDLE_SIZE;

        let near_top = position.y <= top + RESIZE_HANDLE_SIZE;

        let near_bottom = position.y >= bottom - RESIZE_HANDLE_SIZE;

        /*
         * 四隅を辺より先に判定する。
         */
        if near_left && near_top {
            return Some(ResizeEdge::TopLeft);
        }

        if near_right && near_top {
            return Some(ResizeEdge::TopRight);
        }

        if near_left && near_bottom {
            return Some(ResizeEdge::BottomLeft);
        }

        if near_right && near_bottom {
            return Some(ResizeEdge::BottomRight);
        }

        if near_left {
            return Some(ResizeEdge::Left);
        }

        if near_right {
            return Some(ResizeEdge::Right);
        }

        if near_top {
            return Some(ResizeEdge::Top);
        }

        if near_bottom {
            return Some(ResizeEdge::Bottom);
        }

        None
    }

    fn cursor_for_resize_edge(edge: ResizeEdge) -> CursorIcon {
        match edge {
            ResizeEdge::Left | ResizeEdge::Right => CursorIcon::EwResize,

            ResizeEdge::Top | ResizeEdge::Bottom => CursorIcon::NsResize,

            ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorIcon::NwseResize,

            ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorIcon::NeswResize,
        }
    }

    fn topmost_resize_window_at(
        desktop: &DesktopWindows,
        position: Point,
    ) -> Option<(DesktopWindow, ResizeEdge)> {
        desktop.windows.iter().rev().find_map(|window| {
            if window.minimized || !window.resizable || window.restore_frame.is_some() {
                return None;
            }

            Self::resize_edge_at(window.frame, position).map(|edge| (window.clone(), edge))
        })
    }

    fn update_resize_cursor(
        &self,
        position: Point,
        context: &mut EventContext<'_>,
    ) -> Option<ResizeEdge> {
        let desktop = self.windows.get();

        let edge = Self::topmost_resize_window_at(&desktop, position).map(|(_, edge)| edge);

        let cursor = match edge {
            Some(edge) => Self::cursor_for_resize_edge(edge),

            None => CursorIcon::Default,
        };

        context.set_cursor(cursor);

        edge
    }

    fn resize_window(&self, bounds: Rect, resize: WindowResize, pointer: Point) {
        let delta_x = pointer.x - resize.pointer_origin.x;

        let delta_y = pointer.y - resize.pointer_origin.y;

        let work_area = Self::desktop_work_area(bounds);

        let work_left = work_area.origin.x;

        let work_top = work_area.origin.y;

        let work_right = work_area.origin.x + work_area.size.width;

        let work_bottom = work_area.origin.y + work_area.size.height;

        let start_left = resize.frame_origin.x;

        let start_top = resize.frame_origin.y;

        let start_right = resize.frame_origin.x + resize.frame_size.width;

        let start_bottom = resize.frame_origin.y + resize.frame_size.height;

        let resize_left = matches!(
            resize.edge,
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
        );

        let resize_right = matches!(
            resize.edge,
            ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
        );

        let resize_top = matches!(
            resize.edge,
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
        );

        let resize_bottom = matches!(
            resize.edge,
            ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
        );

        let mut left = start_left;
        let mut top = start_top;
        let mut right = start_right;
        let mut bottom = start_bottom;

        if resize_left {
            let maximum_left = (start_right - MINIMUM_WINDOW_WIDTH).max(work_left);

            left = (start_left + delta_x).clamp(work_left, maximum_left);
        }

        if resize_right {
            let minimum_right = (start_left + MINIMUM_WINDOW_WIDTH).min(work_right);

            right = (start_right + delta_x).clamp(minimum_right, work_right);
        }

        if resize_top {
            let maximum_top = (start_bottom - MINIMUM_WINDOW_HEIGHT).max(work_top);

            top = (start_top + delta_y).clamp(work_top, maximum_top);
        }

        if resize_bottom {
            let minimum_bottom = (start_top + MINIMUM_WINDOW_HEIGHT).min(work_bottom);

            bottom = (start_bottom + delta_y).clamp(minimum_bottom, work_bottom);
        }

        let width = (right - left).max(MINIMUM_WINDOW_WIDTH);

        let height = (bottom - top).max(MINIMUM_WINDOW_HEIGHT);

        self.windows.update(|desktop| {
            let Some(window) = desktop
                .windows
                .iter_mut()
                .find(|window| window.id == resize.window)
            else {
                return;
            };

            if !window.resizable || window.restore_frame.is_some() {
                return;
            }

            window.frame = Rect::new(left, top, width, height);
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
        /*
         * リサイズ操作中。
         */
        if let Some(resize) = self.resize.get() {
            return match event {
                ViewEvent::PointerMoved { position } => {
                    context.set_cursor(Self::cursor_for_resize_edge(resize.edge));

                    self.resize_window(bounds, resize, *position);

                    context.request_redraw();

                    EventResult::Consumed
                }

                ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                } => {
                    self.resize.set(None);

                    self.update_resize_cursor(*position, context);

                    context.request_redraw();

                    EventResult::Consumed
                }

                ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                    self.resize.set(None);

                    context.set_cursor(CursorIcon::Default);

                    context.request_redraw();

                    EventResult::Consumed
                }

                _ => EventResult::Consumed,
            };
        }

        /*
         * ウィンドウ移動中。
         */
        if let Some(drag) = self.drag.get() {
            return match event {
                ViewEvent::PointerMoved { position } => {
                    context.set_cursor(CursorIcon::Default);

                    self.move_dragged_window(bounds, drag, *position);

                    context.request_redraw();

                    EventResult::Consumed
                }

                ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                } => {
                    self.drag.set(None);

                    self.update_resize_cursor(*position, context);

                    context.request_redraw();

                    EventResult::Consumed
                }

                ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                    self.drag.set(None);

                    context.set_cursor(CursorIcon::Default);

                    context.request_redraw();

                    EventResult::Consumed
                }

                _ => EventResult::Consumed,
            };
        }

        match event {
            ViewEvent::PointerMoved { position } => {
                let resize_edge = self.update_resize_cursor(*position, context);

                if self.update_hovered_control(*position) {
                    context.request_redraw();
                }

                let desktop = self.windows.get();

                let window_hovered = Self::topmost_window_at(&desktop, *position).is_some();

                if resize_edge.is_some() || window_hovered {
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
                /*
                 * リサイズ領域は通常の
                 * 角丸ヒットテストより先に調べる。
                 */
                let resize_target = {
                    let desktop = self.windows.get();

                    Self::topmost_resize_window_at(&desktop, *position)
                };

                if let Some((resize_window, edge)) = resize_target {
                    context.set_cursor(Self::cursor_for_resize_edge(edge));

                    self.windows.update(|desktop| {
                        desktop.focus(resize_window.id);

                        desktop.clear_pressed_controls();
                    });

                    self.resize.set(Some(WindowResize {
                        window: resize_window.id,

                        edge,

                        pointer_origin: *position,

                        frame_origin: resize_window.frame.origin,

                        frame_size: resize_window.frame.size,
                    }));

                    self.drag.set(None);

                    context.request_redraw();

                    return EventResult::Consumed;
                }

                context.set_cursor(CursorIcon::Default);

                let hit_window = {
                    let desktop = self.windows.get();

                    Self::topmost_window_at(&desktop, *position)
                };

                let Some(hit_window) = hit_window else {
                    let is_desktop_area = position.y >= bounds.origin.y + DESKTOP_TOP_INSET;

                    /*
                     * トップバーを押したときは
                     * Window関連Stateを変更しない。
                     */
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

                    self.resize.set(None);
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

                    self.update_resize_cursor(*position, context);

                    context.request_redraw();

                    return EventResult::Consumed;
                }

                let resize_edge = self.update_resize_cursor(*position, context);

                let desktop = self.windows.get();

                let window_hovered = Self::topmost_window_at(&desktop, *position).is_some();

                if resize_edge.is_some() || window_hovered {
                    EventResult::Consumed
                } else {
                    drop(desktop);

                    self.content.handle_event(bounds, event, context)
                }
            }

            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                context.set_cursor(CursorIcon::Default);

                if self.clear_interactions() {
                    context.request_redraw();
                }

                self.content.handle_event(bounds, event, context)
            }

            _ => {
                if let Some(position) = event.position() {
                    let desktop = self.windows.get();

                    let resize_hit = Self::topmost_resize_window_at(&desktop, position).is_some();

                    let window_hit = Self::topmost_window_at(&desktop, position).is_some();

                    if resize_hit || window_hit {
                        return EventResult::Consumed;
                    }
                }

                self.content.handle_event(bounds, event, context)
            }
        }
    }
}
