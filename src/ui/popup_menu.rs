use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

use viewkit::{
    animation::VisibilityTransition,
    draw_command::PaintTransform,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

#[derive(Default)]
pub(crate) struct PopupMenuInteraction {
    transition: RefCell<VisibilityTransition>,
    presented: Cell<Option<PaintTransform>>,
}

fn presentation(bounds: Rect, amount: f32, closed_scale: f32) -> PaintTransform {
    PaintTransform {
        anchor: bounds.origin,
        scale: closed_scale + (1.0 - closed_scale) * amount,
        offset: Point::new(0.0, 0.0),
        opacity: amount,
    }
}

pub(super) fn logical_event(event: &ViewEvent, transform: PaintTransform) -> ViewEvent {
    let mut event = event.clone();
    match &mut event {
        ViewEvent::PointerMoved { position }
        | ViewEvent::PointerPressed { position, .. }
        | ViewEvent::PointerReleased { position, .. }
        | ViewEvent::PointerFocusRequested { position }
        | ViewEvent::Scroll { position, .. } => *position = transform.inverse_point(*position),
        ViewEvent::FileDragEntered { position, .. } | ViewEvent::FileDropped { position, .. } => {
            *position = position.map(|point| transform.inverse_point(point));
        }
        _ => {}
    }
    event
}

pub(crate) struct PopupMenu<C> {
    content: C,
    menu: Menu,

    open: State<bool>,
    interaction: Rc<PopupMenuInteraction>,

    menu_frame: Rect,
    trigger_frame: Rect,
}

impl<C> PopupMenu<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        menu: Menu,
        open: State<bool>,
        interaction: Rc<PopupMenuInteraction>,
    ) -> Self {
        Self {
            content,
            menu,
            open,
            interaction,

            menu_frame: Rect::new(
                super::desktop_metrics::LOGO_INSET,
                super::desktop_metrics::OS_MENU_TOP,
                super::desktop_metrics::OS_MENU_WIDTH,
                super::desktop_metrics::OS_MENU_HEIGHT,
            ),

            trigger_frame: Rect::new(5.0, 5.0, 44.0, 44.0),
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

    fn redraw_frame(&self, bounds: Rect, theme: &viewkit::theme::Theme) -> Rect {
        let margin = theme
            .shadows
            .card
            .layers
            .iter()
            .flatten()
            .map(|shadow| {
                shadow.blur_radius
                    + shadow.spread.max(0.0)
                    + shadow.offset_x.abs().max(shadow.offset_y.abs())
            })
            .fold(0.0_f32, f32::max)
            + 2.0;
        Self::absolute_frame(bounds, self.trigger_frame)
            .union(Self::absolute_frame(bounds, self.menu_frame).expanded(margin))
    }
}

impl<C> View for PopupMenu<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);

        let open = self.open.get();
        let (amount, next) = self.interaction.transition.borrow_mut().sample(
            open,
            if open {
                context.theme.motion.menu
            } else {
                context.theme.motion.menu_dismiss
            },
            Instant::now(),
        );
        if let Some(deadline) = next {
            context.request_redraw_in_at(self.redraw_frame(bounds, context.theme), deadline);
        }
        let menu_bounds = Self::absolute_frame(bounds, self.menu_frame);
        let transform = presentation(menu_bounds, amount, context.theme.motion.menu_closed_scale);
        self.interaction.presented.set(Some(transform));
        if amount <= 0.0 {
            return;
        }
        context.with_presentation(transform, open, |context| {
            self.menu.paint(menu_bounds, context)
        });
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.open.get() {
            let result = self.content.handle_event(bounds, event, context);
            if self.open.get() {
                context.request_redraw_in(self.redraw_frame(bounds, context.theme()));
            }
            return result;
        }

        let menu_bounds = Self::absolute_frame(bounds, self.menu_frame);

        let trigger_bounds = Self::absolute_frame(bounds, self.trigger_frame);
        let transform = self.interaction.presented.get().unwrap_or_else(|| {
            presentation(menu_bounds, 1.0, context.theme().motion.menu_closed_scale)
        });
        let menu_event = logical_event(event, transform);

        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
                context.request_redraw_in(self.redraw_frame(bounds, context.theme()));
                EventResult::Consumed
            }

            ViewEvent::FocusChanged { focused: false } => {
                self.open.set(false);
                context.request_redraw_in(self.redraw_frame(bounds, context.theme()));
                EventResult::Consumed
            }

            ViewEvent::PointerFocusRequested { .. } => EventResult::Consumed,
            ViewEvent::KeyboardFocusRequested { .. } | ViewEvent::KeyPressed { .. } => self
                .menu
                .handle_event(menu_bounds, &menu_event, context)
                .merge(EventResult::Consumed),

            ViewEvent::PointerMoved { position } => {
                if self.menu.contains(
                    menu_bounds,
                    transform.inverse_point(*position),
                    context.theme(),
                ) {
                    let result = self.menu.handle_event(menu_bounds, &menu_event, context);

                    return result.merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                self.menu.handle_event(menu_bounds, &menu_event, context);

                EventResult::Consumed
            }

            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if self.menu.contains(
                    menu_bounds,
                    transform.inverse_point(*position),
                    context.theme(),
                ) {
                    return self
                        .menu
                        .handle_event(menu_bounds, &menu_event, context)
                        .merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                if position.y < bounds.origin.y + super::desktop_metrics::SYSTEM_HEIGHT {
                    return self.content.handle_event(bounds, event, context);
                }
                self.open.set(false);
                context.request_redraw_in(self.redraw_frame(bounds, context.theme()));
                EventResult::Consumed
            }

            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                if self.menu.contains(
                    menu_bounds,
                    transform.inverse_point(*position),
                    context.theme(),
                ) {
                    let result = self.menu.handle_event(menu_bounds, &menu_event, context);

                    if !self.open.get() {
                        context.request_redraw_in(self.redraw_frame(bounds, context.theme()));
                    }

                    return result.merge(EventResult::Consumed);
                }

                if trigger_bounds.contains(*position) {
                    return self.content.handle_event(bounds, event, context);
                }

                if position.y < bounds.origin.y + super::desktop_metrics::SYSTEM_HEIGHT {
                    return self.content.handle_event(bounds, event, context);
                }
                EventResult::Consumed
            }

            ViewEvent::PointerLeft => self
                .menu
                .handle_event(menu_bounds, &menu_event, context)
                .merge(EventResult::Consumed),

            _ => {
                if let Some(position) = event.position() {
                    if self.menu.contains(
                        menu_bounds,
                        transform.inverse_point(position),
                        context.theme(),
                    ) {
                        return self
                            .menu
                            .handle_event(menu_bounds, &menu_event, context)
                            .merge(EventResult::Consumed);
                    }
                }

                EventResult::Consumed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewkit::{
        accessibility::AccessibilityNode, draw_command::DisplayList, theme::Theme,
        typography::TextMeasurer, view::RedrawSchedule,
    };

    #[test]
    fn closing_keeps_painting_but_releases_focus_and_final_frame_disappears() {
        let theme = Theme::current();
        let open = State::new(true);
        let interaction = Rc::new(PopupMenuInteraction::default());
        let now = Instant::now();
        // Start well in the past so the first assertion is independent of wall-clock speed.
        interaction.transition.borrow_mut().sample(
            true,
            theme.motion.menu,
            now - theme.motion.menu.duration * 2,
        );
        let popup = PopupMenu::new(
            Rectangle::new(),
            Menu::new().compact().item(MenuItem::new("Example")),
            open.clone(),
            Rc::clone(&interaction),
        );
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut measurer = TextMeasurer::new();
        let paint = |list: &mut DisplayList,
                     nodes: &mut Vec<AccessibilityNode>,
                     schedule: &mut RedrawSchedule,
                     measurer: &mut TextMeasurer| {
            popup.paint(
                bounds,
                &mut PaintContext::new(list, &theme, &theme.typography, measurer)
                    .with_accessibility_nodes(nodes)
                    .with_redraw_schedule(schedule),
            );
        };
        let mut list = DisplayList::new();
        let mut nodes = Vec::new();
        let mut schedule = RedrawSchedule::new();
        paint(&mut list, &mut nodes, &mut schedule, &mut measurer);
        let open_commands = list.commands().len();
        assert!(nodes.iter().any(|node| node.focus_scope));
        assert!(schedule.deadline().is_none());
        open.set(false);
        list.clear();
        nodes.clear();
        paint(&mut list, &mut nodes, &mut schedule, &mut measurer);
        assert_eq!(list.commands().len(), open_commands);
        assert!(nodes.is_empty());
        assert!(schedule.deadline().is_some());
        // Finish the close deterministically, with the same persistent state a rebuilt root uses.
        let mut transition = VisibilityTransition::default();
        transition.sample(
            true,
            theme.motion.menu,
            now - theme.motion.menu.duration * 3,
        );
        transition.sample(
            false,
            theme.motion.menu_dismiss,
            now - theme.motion.menu.duration,
        );
        *interaction.transition.borrow_mut() = transition;
        list.clear();
        nodes.clear();
        schedule = RedrawSchedule::new();
        paint(&mut list, &mut nodes, &mut schedule, &mut measurer);
        assert_eq!(list.commands().len(), 1); // Desktop only.
        assert!(schedule.deadline().is_none());
    }

    #[test]
    fn pointer_coordinates_follow_presentation_while_keyboard_focus_stays_stable() {
        let bounds = Rect::new(15.0, 47.0, 150.0, 144.0);
        let transform = presentation(bounds, 0.5, 0.92);
        let logical = Point::new(55.0, 90.0);
        let event = ViewEvent::PointerReleased {
            position: transform.point(logical),
            button: PointerButton::Primary,
        };
        let mapped = logical_event(&event, transform).position().unwrap();
        assert!((mapped.x - logical.x).abs() < 0.001);
        assert!((mapped.y - logical.y).abs() < 0.001);
        let focus = ViewEvent::KeyboardFocusRequested {
            bounds: Some(bounds),
        };
        assert_eq!(logical_event(&focus, transform), focus);
        // Semantic commands already use the dispatcher's logical focus coordinates.
        let command = ViewEvent::Command {
            command: viewkit::command::CommandId::new("example"),
            target: Some(logical),
        };
        assert_eq!(logical_event(&command, transform), command);
        assert_eq!(presentation(bounds, 1.0, 0.92).rect(bounds), bounds);
        assert_eq!(presentation(bounds, 0.0, 0.92).opacity, 0.0);
    }
    #[test]
    fn clicking_a_scaled_item_uses_its_visible_position() {
        let theme = Theme::current();
        let calls = Rc::new(Cell::new(0));
        let callback = Rc::clone(&calls);
        let interaction = Rc::new(PopupMenuInteraction::default());
        let popup = PopupMenu::new(
            Rectangle::new(),
            Menu::new()
                .compact()
                .item(MenuItem::new("Example").on_select(move || callback.set(callback.get() + 1))),
            State::new(true),
            Rc::clone(&interaction),
        );
        let transform = presentation(popup.menu_frame, 0.5, theme.motion.menu_closed_scale);
        interaction.presented.set(Some(transform));
        let logical = Point::new(
            popup.menu_frame.origin.x + theme.menu.compact_padding + 0.1,
            popup.menu_frame.origin.y
                + theme.menu.compact_padding
                + theme.menu.compact_row_height * 0.5,
        );
        let visible = transform.point(logical);
        // The visible left edge falls outside the untransformed item.
        assert!(visible.x < logical.x - 0.1);
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        popup.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position: visible,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        popup.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position: visible,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert_eq!(calls.get(), 1);
    }
}
