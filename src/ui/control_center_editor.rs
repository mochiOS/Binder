use std::cell::RefCell;
use std::rc::Rc;

use crate::control_center_preferences::ControlCenterPreferences;
use crate::platform::AppInfo;
use viewkit::{
    draw_command::DrawCommand,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

use super::control_center::{CustomizationItem, customization_items};

const CONTENT_PADDING: f32 = 22.0;
const INTRO_HEIGHT: f32 = 68.0;
const ROW_HEIGHT: f32 = 58.0;
const ROW_GAP: f32 = 8.0;
const CONTROL_SIZE: f32 = 32.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum EditorHit {
    Row(String),
    MoveUp(String),
    MoveDown(String),
    Toggle(String),
}

#[derive(Default)]
pub(crate) struct ControlCenterEditorInteraction {
    hovered: Option<EditorHit>,
    pressed: Option<EditorHit>,
    scroll_offset: f32,
    cached_apps: Vec<AppInfo>,
    cached_items: Vec<CustomizationItem>,
}

pub(crate) fn view(
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    interaction: Rc<RefCell<ControlCenterEditorInteraction>>,
    apps: State<Vec<AppInfo>>,
) -> impl View + 'static {
    ControlCenterEditorView {
        preferences,
        interaction,
        apps,
    }
}

struct ControlCenterEditorView {
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    interaction: Rc<RefCell<ControlCenterEditorInteraction>>,
    apps: State<Vec<AppInfo>>,
}

impl ControlCenterEditorView {
    fn items(&self) -> Vec<(CustomizationItem, bool)> {
        let apps = self.apps.get();
        let mut interaction = self.interaction.borrow_mut();
        if interaction.cached_apps != apps {
            interaction.cached_items = customization_items(&apps);
            interaction.cached_apps = apps;
        }
        let ordered = self
            .preferences
            .borrow()
            .ordered_ids(interaction.cached_items.iter().map(|item| item.id.as_str()));
        let preferences = self.preferences.borrow();
        let mut items = interaction.cached_items.clone();
        items.sort_by_key(|item| {
            ordered
                .iter()
                .position(|candidate| candidate == &item.id)
                .unwrap_or(usize::MAX)
        });
        items
            .into_iter()
            .map(|item| {
                let hidden = preferences.is_hidden(&item.id);
                (item, hidden)
            })
            .collect()
    }

    fn list_bounds(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x + CONTENT_PADDING,
            bounds.origin.y + INTRO_HEIGHT,
            (bounds.size.width - CONTENT_PADDING * 2.0).max(0.0),
            (bounds.size.height - INTRO_HEIGHT - CONTENT_PADDING).max(0.0),
        )
    }

    fn content_height(item_count: usize) -> f32 {
        item_count as f32 * (ROW_HEIGHT + ROW_GAP) - if item_count > 0 { ROW_GAP } else { 0.0 }
    }

    fn maximum_scroll(list: Rect, item_count: usize) -> f32 {
        (Self::content_height(item_count) - list.size.height).max(0.0)
    }

    fn row_bounds(list: Rect, index: usize, scroll_offset: f32) -> Rect {
        Rect::new(
            list.origin.x,
            list.origin.y + index as f32 * (ROW_HEIGHT + ROW_GAP) - scroll_offset,
            list.size.width - 8.0,
            ROW_HEIGHT,
        )
    }

    fn button_bounds(row: Rect, slot: usize) -> Rect {
        let toggle_left = row.origin.x + row.size.width - 82.0;
        Rect::new(
            toggle_left - (2 - slot.min(1)) as f32 * (CONTROL_SIZE + 6.0),
            row.origin.y + (row.size.height - CONTROL_SIZE) / 2.0,
            CONTROL_SIZE,
            CONTROL_SIZE,
        )
    }

    fn toggle_bounds(row: Rect) -> Rect {
        Rect::new(
            row.origin.x + row.size.width - 82.0,
            row.origin.y + (row.size.height - CONTROL_SIZE) / 2.0,
            70.0,
            CONTROL_SIZE,
        )
    }

    fn hit(&self, bounds: Rect, position: Point) -> Option<EditorHit> {
        let items = self.items();
        let list = Self::list_bounds(bounds);
        if !list.contains(position) {
            return None;
        }
        let maximum = Self::maximum_scroll(list, items.len());
        let offset = self.interaction.borrow().scroll_offset.clamp(0.0, maximum);
        for (index, (item, _)) in items.iter().enumerate() {
            let row = Self::row_bounds(list, index, offset);
            if row.intersection(list).is_none() || !row.contains(position) {
                continue;
            }
            if Self::toggle_bounds(row).contains(position) {
                return Some(EditorHit::Toggle(item.id.clone()));
            }
            if Self::button_bounds(row, 0).contains(position) {
                return Some(EditorHit::MoveUp(item.id.clone()));
            }
            if Self::button_bounds(row, 1).contains(position) {
                return Some(EditorHit::MoveDown(item.id.clone()));
            }
            return Some(EditorHit::Row(item.id.clone()));
        }
        None
    }

    fn save_preferences(&self) {
        if let Err(error) = self.preferences.borrow().save() {
            eprintln!("failed to save Control Center preferences: {error}");
        }
    }

    fn apply(&self, pressed: EditorHit, released: EditorHit) -> bool {
        let items = self.items();
        let available = items
            .iter()
            .map(|(item, _)| item.id.clone())
            .collect::<Vec<_>>();
        let changed = match (pressed, released) {
            (EditorHit::Toggle(from), EditorHit::Toggle(to)) if from == to => {
                let hidden = self.preferences.borrow().is_hidden(&from);
                self.preferences.borrow_mut().set_hidden(&from, !hidden)
            }
            (EditorHit::MoveUp(from), EditorHit::MoveUp(to)) if from == to => {
                self.preferences.borrow_mut().move_by(&from, -1, &available)
            }
            (EditorHit::MoveDown(from), EditorHit::MoveDown(to)) if from == to => {
                self.preferences.borrow_mut().move_by(&from, 1, &available)
            }
            (EditorHit::Row(from), EditorHit::Row(to)) if from != to => self
                .preferences
                .borrow_mut()
                .move_before(&from, &to, &available),
            _ => false,
        };
        if changed {
            self.save_preferences();
        }
        changed
    }

    fn paint_control(
        &self,
        bounds: Rect,
        hit: EditorHit,
        symbol: SymbolName,
        enabled: bool,
        context: &mut PaintContext<'_>,
    ) {
        let interaction = self.interaction.borrow();
        let hovered = interaction.hovered.as_ref() == Some(&hit);
        let pressed = interaction.pressed.as_ref() == Some(&hit) && hovered;
        Rectangle::new()
            .color(RectangleColor::Custom(if !enabled {
                Theme::current().shell.item_enabled.with_alpha(90)
            } else if pressed {
                Theme::current().shell.action_hover
            } else if hovered {
                Theme::current().shell.control_hover
            } else {
                Theme::current().shell.item_enabled
            }))
            .radius(CornerRadius::Medium)
            .paint(bounds, context);
        Icon::new(symbol)
            .size(14.0)
            .color(if enabled {
                Theme::current().shell.primary_text
            } else {
                Theme::current().shell.secondary_text.with_alpha(100)
            })
            .paint(bounds, context);
    }
}

impl View for ControlCenterEditorView {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(540.0, 550.0))
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Text::styled("Choose and arrange controls", TextRole::TitleMedium)
            .weight(700)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    bounds.origin.x + CONTENT_PADDING,
                    bounds.origin.y + 16.0,
                    bounds.size.width - CONTENT_PADDING * 2.0,
                    24.0,
                ),
                context,
            );
        Text::styled(
            "Drag a row or use the arrows. Changes are saved automatically.",
            TextRole::Caption,
        )
        .color(Theme::current().shell.secondary_text)
        .paint(
            Rect::new(
                bounds.origin.x + CONTENT_PADDING,
                bounds.origin.y + 42.0,
                bounds.size.width - CONTENT_PADDING * 2.0,
                20.0,
            ),
            context,
        );

        let items = self.items();
        let list = Self::list_bounds(bounds);
        let maximum = Self::maximum_scroll(list, items.len());
        let offset = self.interaction.borrow().scroll_offset.clamp(0.0, maximum);
        self.interaction.borrow_mut().scroll_offset = offset;
        context
            .display_list
            .push(DrawCommand::PushClip { rect: list });
        for (index, (item, hidden)) in items.iter().enumerate() {
            let row = Self::row_bounds(list, index, offset);
            if row.intersection(list).is_none() {
                continue;
            }
            let row_hit = EditorHit::Row(item.id.clone());
            let row_hovered = self.interaction.borrow().hovered.as_ref() == Some(&row_hit);
            Rectangle::new()
                .color(RectangleColor::Custom(if row_hovered {
                    Theme::current().shell.item_hover
                } else {
                    Theme::current().shell.item_enabled
                }))
                .radius(CornerRadius::Large)
                .border(BorderStyle::custom(
                    Theme::current().shell.panel_border,
                    1.0,
                ))
                .paint(row, context);

            let icon = Rect::new(row.origin.x + 12.0, row.origin.y + 9.0, 40.0, 40.0);
            Rectangle::new()
                .color(RectangleColor::Custom(if *hidden {
                    Theme::current().shell.control_hover
                } else {
                    Theme::current().colors.accent
                }))
                .radius(CornerRadius::Medium)
                .paint(icon, context);
            Icon::new(item.symbol)
                .size(18.0)
                .color(if *hidden {
                    Theme::current().shell.primary_text
                } else {
                    Color::WHITE
                })
                .paint(icon, context);
            Text::styled(item.title.clone(), TextRole::Label)
                .weight(650)
                .color(Theme::current().shell.primary_text)
                .paint(
                    Rect::new(row.origin.x + 64.0, row.origin.y + 8.0, 190.0, 22.0),
                    context,
                );
            Text::styled(item.source.clone(), TextRole::Caption)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(row.origin.x + 64.0, row.origin.y + 30.0, 190.0, 18.0),
                    context,
                );

            let up = Self::button_bounds(row, 0);
            let down = Self::button_bounds(row, 1);
            self.paint_control(
                up,
                EditorHit::MoveUp(item.id.clone()),
                SymbolName::ChevronTop,
                index > 0,
                context,
            );
            self.paint_control(
                down,
                EditorHit::MoveDown(item.id.clone()),
                SymbolName::ChevronDown,
                index + 1 < items.len(),
                context,
            );

            let toggle = Self::toggle_bounds(row);
            let toggle_hit = EditorHit::Toggle(item.id.clone());
            let toggle_hovered = self.interaction.borrow().hovered.as_ref() == Some(&toggle_hit);
            Rectangle::new()
                .color(RectangleColor::Custom(if toggle_hovered {
                    Theme::current().shell.control_hover
                } else {
                    Theme::current().shell.item_enabled
                }))
                .radius(CornerRadius::Medium)
                .paint(toggle, context);
            Icon::new(if *hidden {
                SymbolName::Plus
            } else {
                SymbolName::Minus
            })
            .size(13.0)
            .color(if *hidden {
                Theme::current().colors.accent
            } else {
                Theme::current().shell.alert
            })
            .paint(
                Rect::new(
                    toggle.origin.x + 6.0,
                    toggle.origin.y,
                    22.0,
                    toggle.size.height,
                ),
                context,
            );
            Text::styled(if *hidden { "Add" } else { "Hide" }, TextRole::Caption)
                .weight(650)
                .color(Theme::current().shell.primary_text)
                .alignment(TextAlignment::Center)
                .paint(
                    Rect::new(
                        toggle.origin.x + 25.0,
                        toggle.origin.y,
                        toggle.size.width - 29.0,
                        toggle.size.height,
                    ),
                    context,
                );
        }
        context.display_list.push(DrawCommand::PopClip);

        if maximum > 0.0 {
            let thumb_height = (list.size.height * list.size.height
                / Self::content_height(items.len()))
            .max(32.0)
            .min(list.size.height);
            let thumb_y = list.origin.y + (list.size.height - thumb_height) * offset / maximum;
            Rectangle::new()
                .color(RectangleColor::Custom(
                    Theme::current().scrollbar.thumb_color,
                ))
                .radius(CornerRadius::Full)
                .paint(
                    Rect::new(
                        list.origin.x + list.size.width - 4.0,
                        thumb_y,
                        4.0,
                        thumb_height,
                    ),
                    context,
                );
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        match event {
            ViewEvent::PointerMoved { position } => {
                let hit = self.hit(bounds, *position);
                if self.interaction.borrow().hovered != hit {
                    self.interaction.borrow_mut().hovered = hit;
                    context.request_redraw_in(bounds);
                }
            }
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                let mut interaction = self.interaction.borrow_mut();
                interaction.hovered = None;
                interaction.pressed = None;
                context.request_redraw_in(bounds);
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                let hit = self.hit(bounds, *position);
                self.interaction.borrow_mut().pressed = hit;
                context.request_redraw_in(bounds);
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let released = self.hit(bounds, *position);
                let pressed = self.interaction.borrow_mut().pressed.take();
                if let (Some(pressed), Some(released)) = (pressed, released) {
                    self.apply(pressed, released);
                }
                context.request_redraw_in(bounds);
            }
            ViewEvent::Scroll {
                position, delta_y, ..
            } if Self::list_bounds(bounds).contains(*position) => {
                let item_count = self.items().len();
                let list = Self::list_bounds(bounds);
                let maximum = Self::maximum_scroll(list, item_count);
                let mut interaction = self.interaction.borrow_mut();
                interaction.scroll_offset =
                    (interaction.scroll_offset - *delta_y).clamp(0.0, maximum);
                context.request_redraw_in(list);
            }
            _ => {}
        }
        EventResult::Consumed
    }
}
