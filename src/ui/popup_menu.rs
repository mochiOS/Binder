use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) struct PopupMenu<C, M> {
    content: C,
    menu: M,

    open: State<bool>,

    menu_frame: Rect,
    trigger_frame: Rect,
}

impl<C, M> PopupMenu<C, M>
where
    C: View,
    M: View,
{
    pub(crate) fn new(content: C, menu: M, open: State<bool>) -> Self {
        Self {
            content,
            menu,
            open,

            menu_frame: Rect::new(14.0, 42.0, 250.0, 286.0),

            trigger_frame: Rect::new(14.0, 5.0, 92.0, 29.0),
        }
    }

    fn absolute_frame(bounds: Rect, frame: Rect) -> Rect {
        Rect::new(
            bounds.origin.x + frame.origin.x,
            bounds.origin.y + frame.origin.y,
            frame.size.width,
            frame.size.height,
        )
    }
}

impl<C, M> View for PopupMenu<C, M>
where
    C: View,
    M: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);

        if !self.open.get() {
            return;
        }

        let menu_bounds = Self::absolute_frame(bounds, self.menu_frame);

        self.menu.paint(menu_bounds, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.open.get() {
            return self.content.handle_event(bounds, event, context);
        }

        let menu_bounds = Self::absolute_frame(bounds, self.menu_frame);

        let trigger_bounds = Self::absolute_frame(bounds, self.trigger_frame);

        match event {
            ViewEvent::FocusChanged { focused: false } => {
                self.open.set(false);
                context.request_redraw();

                EventResult::Consumed
            }

            ViewEvent::PointerFocusRequested { .. } => EventResult::Consumed,

            ViewEvent::PointerMoved { position } => {
                if menu_bounds.contains(*position) {
                    let result = self.menu.handle_event(menu_bounds, event, context);

                    return result.merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                self.menu.handle_event(menu_bounds, event, context);

                EventResult::Consumed
            }

            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if menu_bounds.contains(*position) {
                    return self
                        .menu
                        .handle_event(menu_bounds, event, context)
                        .merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                self.open.set(false);
                context.request_redraw();

                EventResult::Consumed
            }

            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                if menu_bounds.contains(*position) {
                    let result = self.menu.handle_event(menu_bounds, event, context);

                    if result.is_consumed() {
                        self.open.set(false);
                        context.request_redraw();
                    }

                    return result.merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                EventResult::Consumed
            }

            ViewEvent::PointerLeft => self
                .menu
                .handle_event(menu_bounds, event, context)
                .merge(EventResult::Consumed),

            _ => {
                if let Some(position) = event.position() {
                    if menu_bounds.contains(position) {
                        return self
                            .menu
                            .handle_event(menu_bounds, event, context)
                            .merge(EventResult::Consumed);
                    }
                }

                EventResult::Consumed
            }
        }
    }
}
