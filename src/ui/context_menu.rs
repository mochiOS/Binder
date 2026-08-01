use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::platform::{ContextMenuModel, DesktopPlatform};
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const MENU_WIDTH: f32 = 220.0;
const MENU_ITEM_HEIGHT: f32 = 34.0;
const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
const MENU_PADDING: f32 = 12.0;
const MENU_REDRAW_MARGIN: f32 = 16.0;

pub(crate) struct ContextMenuLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    state: State<Option<ContextMenuModel>>,
    menu: Option<Menu>,
    selected: Rc<Cell<Option<u32>>>,
}

impl<C> ContextMenuLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        state: State<Option<ContextMenuModel>>,
    ) -> Self {
        let selected = Rc::new(Cell::new(None));
        let menu = state.get().as_ref().map(|model| {
            let mut menu = Menu::new();
            for entry in &model.entries {
                if entry.separator {
                    menu = menu.separator();
                    continue;
                }
                let selected_for_item = Rc::clone(&selected);
                let command_id = entry.command_id;
                let label = if entry.checked {
                    format!("\u{2713} {}", entry.label)
                } else {
                    entry.label.clone()
                };
                menu = menu.item(
                    MenuItem::new(label)
                        .enabled(entry.enabled)
                        .danger(entry.destructive)
                        .on_select(move || selected_for_item.set(Some(command_id))),
                );
            }
            menu
        });
        Self {
            content,
            platform,
            state,
            menu,
            selected,
        }
    }

    fn menu_bounds(&self, bounds: Rect, model: &ContextMenuModel) -> Rect {
        let height = model.entries.iter().fold(MENU_PADDING, |height, entry| {
            height
                + if entry.separator {
                    MENU_SEPARATOR_HEIGHT
                } else {
                    MENU_ITEM_HEIGHT
                }
        });
        let maximum_x = (bounds.origin.x + bounds.size.width - MENU_WIDTH).max(bounds.origin.x);
        let maximum_y = (bounds.origin.y + bounds.size.height - height).max(bounds.origin.y);
        Rect::new(
            model.position.x.clamp(bounds.origin.x, maximum_x),
            model.position.y.clamp(bounds.origin.y, maximum_y),
            MENU_WIDTH,
            height,
        )
    }

    fn complete(
        &self,
        request_id: u64,
        command_id: Option<u32>,
        bounds: Rect,
        menu_bounds: Rect,
        context: &mut EventContext<'_>,
    ) {
        if let Err(error) = self
            .platform
            .borrow_mut()
            .complete_context_menu(request_id, command_id)
        {
            eprintln!("failed to complete context menu: {error:?}");
        }
        self.state.set(None);
        self.selected.set(None);
        context.request_redraw_in(
            menu_bounds
                .expanded(MENU_REDRAW_MARGIN)
                .intersection(bounds)
                .unwrap_or(bounds),
        );
    }
}

impl<C> View for ContextMenuLayer<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        let model = self.state.get();
        let (Some(model), Some(menu)) = (model.as_ref(), self.menu.as_ref()) else {
            return;
        };
        menu.paint(self.menu_bounds(bounds, model), context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let model = self.state.get();
        let (Some(model), Some(menu)) = (model.as_ref(), self.menu.as_ref()) else {
            return self.content.handle_event(bounds, event, context);
        };
        let menu_bounds = self.menu_bounds(bounds, model);
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            }
            | ViewEvent::FocusChanged { focused: false } => {
                menu.handle_event(menu_bounds, &ViewEvent::PointerLeft, context);
                self.complete(model.request_id, None, bounds, menu_bounds, context);
                EventResult::Consumed
            }
            ViewEvent::PointerMoved { position } => {
                if menu_bounds.contains(*position) {
                    menu.handle_event(menu_bounds, event, context)
                        .merge(EventResult::Consumed)
                } else {
                    menu.handle_event(menu_bounds, &ViewEvent::PointerLeft, context);
                    EventResult::Consumed
                }
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } if menu_bounds.contains(*position) => menu
                .handle_event(menu_bounds, event, context)
                .merge(EventResult::Consumed),
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } if menu_bounds.contains(*position) => {
                let result = menu.handle_event(menu_bounds, event, context);
                if let Some(command_id) = self.selected.replace(None) {
                    self.complete(
                        model.request_id,
                        Some(command_id),
                        bounds,
                        menu_bounds,
                        context,
                    );
                }
                result.merge(EventResult::Consumed)
            }
            ViewEvent::PointerPressed { .. } => {
                self.complete(model.request_id, None, bounds, menu_bounds, context);
                EventResult::Consumed
            }
            ViewEvent::PointerLeft => menu
                .handle_event(menu_bounds, event, context)
                .merge(EventResult::Consumed),
            _ => EventResult::Consumed,
        }
    }
}
