use super::{
    control_center::ControlCenterInteraction,
    desktop_metrics as metrics,
    status_items::{self, Item, ItemAction},
};
use crate::{
    control_center_preferences::{ControlCenterPreferences, StatusItemPlacement},
    platform::{AppInfo, SystemBarState},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::OnceLock,
};
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

fn retained_button(
    shared: &Rc<RefCell<ControlCenterInteraction>>,
    id: &str,
) -> ButtonInteractionState {
    shared
        .borrow_mut()
        .status_buttons
        .entry(id.into())
        .or_default()
        .clone()
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn view(
    state: State<SystemBarState>,
    menu_open: State<bool>,
    control_open: State<bool>,
    notification_open: State<bool>,
    apps: State<Vec<AppInfo>>,
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    interaction: Rc<RefCell<ControlCenterInteraction>>,
) -> SystemBar {
    let palette = interaction.borrow().status_palette.clone();
    let hovered = interaction.borrow().status_hovered.clone();
    let catalog = status_items::catalog(&apps.get());
    interaction
        .borrow_mut()
        .status_buttons
        .retain(|id, _| id.starts_with("chrome.") || catalog.iter().any(|item| item.id == *id));
    let (menu, controls, notifications) = (
        menu_open.clone(),
        control_open.clone(),
        notification_open.clone(),
    );
    let logo = Button::with_interaction_and_label(retained_button(&interaction, "chrome.logo"), "")
        .content_padding(false)
        .content(Logo)
        .style(ButtonStyle::Ghost)
        .accessibility_label("mochiOS menu")
        .on_click(move || {
            menu.set(!menu.get());
            controls.set(false);
            notifications.set(false);
        });
    let (menu, controls, notifications) = (
        menu_open.clone(),
        control_open.clone(),
        notification_open.clone(),
    );
    let status =
        Button::with_interaction_and_label(retained_button(&interaction, "chrome.background"), "")
            .style(ButtonStyle::Ghost)
            .radius(CornerRadius::Full)
            .shadow(ShadowStyle::None)
            .accessibility_label("Control Center")
            .on_click(move || {
                controls.set(!controls.get());
                menu.set(false);
                notifications.set(false);
            });
    let mut entries = Vec::new();
    for item in catalog {
        let mut button =
            Button::with_interaction_and_label(retained_button(&interaction, &item.id), "")
                .content_padding(false)
                .style(ButtonStyle::Ghost)
                .shadow(ShadowStyle::None)
                .accessibility_label(&item.title);
        if item.action == ItemAction::ToggleInput {
            button = button.content(InputLanguage(state.clone(), palette.clone()));
        } else {
            button = button.content(StatusIcon(
                item.symbol,
                if item.action == ItemAction::ShowBattery {
                    28.0
                } else {
                    20.0
                },
                palette.clone(),
            ));
        }
        let (menu, controls, notifications) = (
            menu_open.clone(),
            control_open.clone(),
            notification_open.clone(),
        );
        let current = state.clone();
        let shared = interaction.clone();
        let action = item.action.clone();
        let id = item.id.clone();
        button = button.on_click(move || match action {
            ItemAction::ToggleInput => {
                if let Some(enabled) = viewkit::platform::input_method::toggle() {
                    current.update(|s| s.japanese_input = enabled);
                }
            }
            ItemAction::ShowNotifications => {
                notifications.set(!notifications.get());
                controls.set(false);
                menu.set(false);
            }
            _ => {
                shared.borrow_mut().pending_status_item = Some(id.clone());
                controls.set(true);
                notifications.set(false);
                menu.set(false);
            }
        });
        entries.push(StatusEntry { item, button });
    }
    let clock =
        Button::with_interaction_and_label(retained_button(&interaction, "chrome.clock"), "")
            .content_padding(false)
            .content(Clock(state, palette.clone()))
            .style(ButtonStyle::Ghost)
            .accessibility_label("Date and time");
    SystemBar {
        logo,
        status,
        clock,
        entries,
        preferences,
        palette,
        hovered,
        presented_frame: Cell::new(None),
    }
}
struct StatusEntry {
    item: Item,
    button: Button,
}
pub(crate) struct SystemBar {
    logo: Button,
    status: Button,
    clock: Button,
    entries: Vec<StatusEntry>,
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    palette: Rc<Cell<ChromePalette>>,
    hovered: Rc<Cell<bool>>,
    presented_frame: Cell<Option<Rect>>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ChromePalette {
    foreground: Color,
    background: Color,
}
impl Default for ChromePalette {
    fn default() -> Self {
        Self {
            foreground: metrics::SYSTEM_FOREGROUND,
            background: Color::rgba(255, 255, 255, 64),
        }
    }
}

struct StatusIcon(SymbolName, f32, Rc<Cell<ChromePalette>>);
impl View for StatusIcon {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Icon::new(self.0)
            .size(self.1)
            .color(self.2.get().foreground)
            .paint(bounds, context);
    }
}
struct StatusLayout {
    pill: Rect,
    date: Rect,
    entries: Vec<(usize, Rect)>,
}
fn logo_frame(bounds: Rect) -> Rect {
    Rect::new(
        bounds.origin.x + metrics::LOGO_INSET - 10.0,
        bounds.origin.y + metrics::LOGO_INSET - 10.0,
        44.0,
        44.0,
    )
}
fn entry_width(item: &Item) -> f32 {
    match item.action {
        ItemAction::ToggleInput => 16.0,
        ItemAction::ShowBattery => 32.0,
        _ => 24.0,
    }
}
fn hit_frame(item: &Item, frame: Rect) -> Rect {
    if item.action == ItemAction::ToggleInput {
        Rect::new(
            frame.origin.x - 4.0,
            frame.origin.y,
            frame.size.width + 8.0,
            frame.size.height,
        )
    } else {
        frame
    }
}
impl SystemBar {
    pub(crate) fn on_clock_click(mut self, callback: impl FnMut() + 'static) -> Self {
        self.clock = self.clock.on_click(callback);
        self
    }
    pub(crate) fn pill(&self, bounds: Rect) -> Rect {
        self.resolved_frames(bounds).pill
    }
    pub(crate) fn set_presented_frame(&self, frame: Option<Rect>) {
        self.presented_frame.set(frame);
    }
    pub(crate) fn sync_pointer(&self, position: Option<Point>) -> Option<Rect> {
        let frame = self.presented_frame.get();
        let hovered = frame.is_some_and(|frame| {
            position.is_some_and(|point| frame.contains_rounded(frame.size.height * 0.5, point))
        });
        (self.hovered.replace(hovered) != hovered)
            .then_some(frame.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)))
    }
    pub(crate) fn is_hovered(&self) -> bool {
        self.hovered.get()
    }
    pub(crate) fn regions(&self, bounds: Rect) -> [Rect; 2] {
        let left = 64.0_f32.min(bounds.size.width);
        let x = (self.resolved_frames(bounds).pill.origin.x - 8.0).max(bounds.origin.x + left);
        [
            Rect::new(bounds.origin.x, bounds.origin.y, left, bounds.size.height),
            Rect::new(
                x,
                bounds.origin.y,
                (bounds.origin.x + bounds.size.width - x).max(0.0),
                bounds.size.height,
            ),
        ]
    }
    pub(crate) fn set_backdrop(&self, background: Color) -> bool {
        let linear = |channel: u8| {
            let value = f32::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance = 0.2126 * linear(background.red)
            + 0.7152 * linear(background.green)
            + 0.0722 * linear(background.blue);
        let threshold = if self.palette.get().foreground == Color::BLACK {
            0.14
        } else {
            0.22
        };
        let dark_text = luminance > threshold;
        let palette = ChromePalette {
            foreground: if dark_text {
                Color::BLACK
            } else {
                Color::WHITE
            },
            background: if dark_text {
                Color::rgba(255, 255, 255, 64)
            } else {
                Color::rgba(0, 0, 0, 96)
            },
        };
        let changed = self.palette.get() != palette;
        self.palette.set(palette);
        changed
    }
    pub(crate) fn is_blank(&self, bounds: Rect, position: Point) -> bool {
        let layout = self.resolved_frames(bounds);
        bounds.contains(position)
            && !logo_frame(bounds).contains(position)
            && !layout.date.contains(position)
            && !layout.entries.iter().any(|(index, frame)| {
                hit_frame(&self.entries[*index].item, *frame).contains(position)
            })
    }
    fn resolved_frames(&self, bounds: Rect) -> StatusLayout {
        let preferences = self.preferences.borrow();
        let ordered =
            preferences.ordered_ids(self.entries.iter().map(|entry| entry.item.id.as_str()));
        let maximum = (bounds.size.width - metrics::CHROME_RIGHT - 70.0).max(0.0);
        let content_start = metrics::STATUS_MENU_SPACE + 104.0;
        let mut cursor = content_start;
        let mut entries = Vec::new();
        for id in ordered {
            let Some((index, entry)) = self
                .entries
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.item.id == id)
            else {
                continue;
            };
            if preferences.placement(&id, entry.item.default_placement)
                != StatusItemPlacement::Pinned
            {
                continue;
            }
            let width = entry_width(&entry.item);
            // Overflow retains its menu entry and saved pin preference.
            if cursor + width + metrics::STATUS_MENU_SPACE > maximum {
                continue;
            }
            entries.push((index, Rect::new(cursor, 0.0, width, metrics::STATUS_HEIGHT)));
            cursor += width + 8.0;
        }
        let width = if entries.is_empty() {
            (content_start + metrics::STATUS_MENU_SPACE).min(maximum)
        } else {
            cursor - 8.0 + metrics::STATUS_MENU_SPACE
        };
        let pill = Rect::new(
            bounds.origin.x + bounds.size.width - metrics::CHROME_RIGHT - width,
            bounds.origin.y + metrics::STATUS_GROUP_TOP,
            width,
            metrics::STATUS_HEIGHT,
        );
        for (_, frame) in &mut entries {
            frame.origin.x += pill.origin.x;
            frame.origin.y = pill.origin.y;
        }
        let date = Rect::new(
            pill.origin.x + metrics::STATUS_MENU_SPACE.min(width),
            pill.origin.y,
            104.0_f32.min((width - metrics::STATUS_MENU_SPACE * 2.0).max(0.0)),
            pill.size.height,
        );
        StatusLayout {
            pill,
            date,
            entries,
        }
    }
}
impl SystemBar {
    pub(crate) fn paint_part(&self, part: usize, bounds: Rect, context: &mut PaintContext<'_>) {
        if part == 0 {
            self.logo.paint(logo_frame(bounds), context);
            return;
        }
        let layout = self.resolved_frames(bounds);
        if self.hovered.get() {
            Rectangle::new()
                .color(RectangleColor::Custom(self.palette.get().background))
                .radius(CornerRadius::Full)
                .paint(layout.pill, context);
        }
        for (index, frame) in layout.entries {
            self.entries[index]
                .button
                .paint(hit_frame(&self.entries[index].item, frame), context);
        }
        self.clock.paint(layout.date, context);
        // Register the background after individual controls, preserving pointer focus priority.
        self.status.paint(layout.pill, context);
    }
    pub(crate) fn handle_part(
        &self,
        part: usize,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if part == 0 {
            return self.logo.handle_event(logo_frame(bounds), event, context);
        }
        let layout = self.resolved_frames(bounds);
        let hovered = match event {
            ViewEvent::PointerMoved { position }
            | ViewEvent::PointerPressed { position, .. }
            | ViewEvent::PointerReleased { position, .. } => Some(
                layout
                    .pill
                    .contains_rounded(layout.pill.size.height * 0.5, *position),
            ),
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => Some(false),
            _ => None,
        };
        if let Some(hovered) = hovered
            && self.hovered.replace(hovered) != hovered
        {
            context.request_redraw_in(layout.pill);
        }
        let reserved = event.position().is_some_and(|p| {
            layout.date.contains(p)
                || layout
                    .entries
                    .iter()
                    .any(|(i, f)| hit_frame(&self.entries[*i].item, *f).contains(p))
        });
        let mut background_event = event.clone();
        let outside_surface = event.position().is_some_and(|p| !layout.pill.contains_rounded(layout.pill.size.height * 0.5, p));
        if reserved || outside_surface {
            let outside = Point::new(bounds.origin.x - 100.0, bounds.origin.y - 100.0);
            match &mut background_event {
                ViewEvent::PointerMoved { position }
                | ViewEvent::PointerPressed { position, .. }
                | ViewEvent::PointerReleased { position, .. } => *position = outside,
                _ => {}
            }
        }
        let mut result = self
            .status
            .handle_event(layout.pill, &background_event, context);
        for (index, frame) in layout.entries {
            result = result.merge(self.entries[index].button.handle_event(
                hit_frame(&self.entries[index].item, frame),
                event,
                context,
            ));
        }
        self.clock
            .handle_event(layout.date, event, context)
            .merge(result)
    }
}
impl View for SystemBar {
    fn measure(&self, c: Constraints, _: &mut MeasureContext<'_>) -> Size {
        c.constrain(Size::new(c.maximum.width, metrics::SYSTEM_HEIGHT))
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.paint_part(0, bounds, context);
        self.paint_part(1, bounds, context);
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        self.handle_part(0, bounds, event, context)
            .merge(self.handle_part(1, bounds, event, context))
    }
}

struct Logo;
impl View for Logo {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(metrics::LOGO_SIZE, metrics::LOGO_SIZE))
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        static LOGO: OnceLock<SvgData> = OnceLock::new();
        let logo = LOGO.get_or_init(|| {
            SvgData::decode(include_bytes!("../../resources/mochios.svg")).expect("mochiOS logo")
        });
        Svg::new(logo.clone()).paint(
            Rect::new(
                bounds.origin.x + (bounds.size.width - metrics::LOGO_SIZE) * 0.5,
                bounds.origin.y + (bounds.size.height - metrics::LOGO_SIZE) * 0.5,
                metrics::LOGO_SIZE,
                metrics::LOGO_SIZE,
            ),
            context,
        );
    }
}
fn label(
    value: impl Into<String>,
    bounds: Rect,
    alignment: TextAlignment,
    center_visible_ink: bool,
    foreground: Color,
    context: &mut PaintContext<'_>,
) {
    let height = metrics::STATUS_LINE_HEIGHT * context.text_measurer.font_scale();
    let text_bounds = if center_visible_ink {
        bounds
    } else {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + (bounds.size.height - height) * 0.5,
            bounds.size.width,
            height,
        )
    };
    Text::styled(value, TextRole::Label)
        .font_size(metrics::STATUS_FONT_SIZE)
        .line_height(metrics::STATUS_LINE_HEIGHT)
        .weight(context.typography.label.weight.0)
        .alignment(alignment)
        .color(foreground)
        .center_visible_ink(center_visible_ink)
        .paint(text_bounds, context);
}
struct InputLanguage(State<SystemBarState>, Rc<Cell<ChromePalette>>);
impl View for InputLanguage {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        label(
            if self.0.get().japanese_input {
                "あ"
            } else {
                "A"
            },
            bounds,
            TextAlignment::Center,
            true,
            self.1.get().foreground,
            context,
        );
    }
}
struct Clock(State<SystemBarState>, Rc<Cell<ChromePalette>>);
impl View for Clock {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let clock = self.0.get().clock;
        label(
            format!("{}, {}", short_date(&clock.date), clock.time),
            bounds,
            TextAlignment::Start,
            true,
            self.1.get().foreground,
            context,
        );
    }
}
fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if let Some((month, day)) = date
        .split_whitespace()
        .next()
        .and_then(|value| value.split_once('/'))
        && let (Ok(month), Ok(day)) = (month.parse::<usize>(), day.parse::<u32>())
        && let Some(month) = month.checked_sub(1).and_then(|index| MONTHS.get(index))
    {
        return format!("{day} {month}");
    }
    date.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use viewkit::{platform::PointerButton, typography::TextMeasurer};
    fn bar(count: Rc<Cell<usize>>) -> SystemBar {
        let entries = status_items::catalog(&[])
            .into_iter()
            .map(|item| StatusEntry {
                item,
                button: Button::new(""),
            })
            .collect();
        SystemBar {
            logo: Button::new(""),
            clock: Button::new(""),
            entries,
            preferences: Rc::new(RefCell::new(ControlCenterPreferences::default())),
            palette: Rc::new(Cell::new(ChromePalette::default())),
            hovered: Rc::new(Cell::new(false)),
            presented_frame: Cell::new(None),
            status: Button::new("")
                .radius(CornerRadius::Full)
                .on_click(move || count.set(count.get() + 1)),
        }
    }
    fn click(bar: &impl View, bounds: Rect, position: Point, context: &mut EventContext<'_>) {
        bar.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            },
            context,
        );
        bar.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            },
            context,
        );
    }
    #[test]
    fn status_surface_is_absent_at_rest_and_only_drawn_during_hover() {
        use viewkit::draw_command::{DisplayList, DrawCommand};
        let bar = bar(Rc::new(Cell::new(0)));
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut list = DisplayList::new();
        let has_plate = |list: &DisplayList| {
            list.commands().iter().any(|command| matches!(command,
            DrawCommand::FillRoundedRect { rect, color, .. } if rect.size.width == metrics::STATUS_GROUP_WIDTH
                && *color == Color::rgba(255, 255, 255, 64)))
        };
        bar.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
        );
        assert!(!has_plate(&list));
        {
            let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
            bar.handle_event(
                bounds,
                &ViewEvent::PointerMoved {
                    position: Point::new(1400.0, 27.0),
                },
                &mut context,
            );
        }
        list.clear();
        bar.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
        );
        assert!(has_plate(&list));
        {
            let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
            bar.handle_event(bounds, &ViewEvent::PointerLeft, &mut context);
        }
        list.clear();
        bar.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
        );
        assert!(!has_plate(&list));
    }

    #[test]
    fn backdrop_changes_native_text_symbols_and_translucent_surface_together() {
        use viewkit::draw_command::{DisplayList, DrawCommand};
        let bar = view(
            State::new(SystemBarState::default()),
            State::new(false),
            State::new(false),
            State::new(false),
            State::new(Vec::new()),
            Rc::new(RefCell::new(ControlCenterPreferences::default())),
            Rc::new(RefCell::new(ControlCenterInteraction::default())),
        );
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut list = DisplayList::new();
        for (background, foreground, surface) in [
            (Color::WHITE, Color::BLACK, Color::rgba(255, 255, 255, 64)),
            (Color::BLACK, Color::WHITE, Color::rgba(0, 0, 0, 96)),
        ] {
            bar.set_backdrop(background);
            bar.hovered.set(true);
            list.clear();
            bar.paint(
                bounds,
                &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
            );
            assert!(list.commands().iter().any(|command| matches!(command, DrawCommand::DrawText { command } if command.color == foreground)));
            assert!(list.commands().iter().any(|command| matches!(command, DrawCommand::DrawSvg { command } if command.tint == Some(foreground))));
            assert!(list.commands().iter().any(|command| matches!(command, DrawCommand::FillRoundedRect { rect, color, .. } if rect.size.width == metrics::STATUS_GROUP_WIDTH && *color == surface)));
        }
    }

    #[test]
    fn default_registration_preserves_positions_and_exact_vertical_center() {
        let bar = bar(Rc::new(Cell::new(0)));
        for bounds in [
            Rect::new(0.0, 0.0, 1440.0, 54.0),
            Rect::new(80.0, 53.5, 1280.0, 90.0),
        ] {
            let layout = bar.resolved_frames(bounds);
            assert_eq!(layout.pill.size.width, metrics::STATUS_GROUP_WIDTH);
            assert_eq!(
                layout
                    .entries
                    .iter()
                    .map(|(_, r)| r.origin.x - layout.pill.origin.x)
                    .collect::<Vec<_>>(),
                [136.0, 160.0, 192.0, 232.0]
            );
            for (_, frame) in layout.entries {
                assert_eq!(
                    frame.origin.y + frame.size.height * 0.5,
                    layout.pill.origin.y + 17.0
                );
            }
        }
    }
    #[test]
    fn background_gaps_open_menu_and_reserved_controls_cancel_a_background_press() {
        let count = Rc::new(Cell::new(0));
        let bar = bar(count.clone());
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let layout = bar.resolved_frames(bounds);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let gap = Point::new(layout.pill.origin.x + 158.0, layout.pill.origin.y + 17.0);
        let ime = Point::new(layout.pill.origin.x + 144.0, layout.pill.origin.y + 17.0);
        bar.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position: gap,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        bar.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position: ime,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert_eq!(count.get(), 0);
        click(&bar, bounds, gap, &mut context);
        assert_eq!(count.get(), 1);
        let menu_space = Point::new(
            layout.pill.origin.x + layout.pill.size.width - metrics::STATUS_MENU_SPACE * 0.5,
            layout.pill.origin.y + 17.0,
        );
        click(&bar, bounds, menu_space, &mut context);
        let left_space = Point::new(
            layout.pill.origin.x + metrics::STATUS_MENU_SPACE * 0.5,
            layout.pill.origin.y + 17.0,
        );
        click(&bar, bounds, left_space, &mut context);
        assert_eq!(count.get(), 3);
        click(
            &bar,
            bounds,
            Point::new(layout.date.origin.x + 30.0, layout.date.origin.y + 17.0),
            &mut context,
        );
        assert_eq!(count.get(), 3);
        click(&bar, bounds, layout.pill.origin, &mut context);
        assert_eq!(count.get(), 3);
    }
    #[test]
    fn menu_and_hidden_placement_remove_icons_and_narrow_windows_keep_overflow_in_registry() {
        let bar = bar(Rc::new(Cell::new(0)));
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        bar.preferences
            .borrow_mut()
            .set_placement("builtin.network", StatusItemPlacement::Menu);
        bar.preferences
            .borrow_mut()
            .set_placement("builtin.battery", StatusItemPlacement::Hidden);
        assert_eq!(bar.resolved_frames(bounds).entries.len(), 2);
        let layout = bar.resolved_frames(Rect::new(0.0, 0.0, 240.0, 54.0));
        assert!(layout.pill.origin.x >= 70.0);
        assert_eq!(status_items::catalog(&[]).len(), 4);
    }
    #[test]
    fn application_registration_can_be_pinned_and_removed_without_changing_its_action() {
        let app = AppInfo {
            root: "/test/Example.app".into(),
            name: "Example".into(),
            bundle_id: "org.mochios.example".into(),
            version: "1".into(),
            developer: "mochiOS".into(),
            entry: "entry.elf".into(),
            description: String::new(),
            icon: None,
            resources: Vec::new(),
        };
        let item = status_items::parse_app_items(
            "format = 1\n[[application.control_center_items]]\nid = \"status\"\ntitle = \"Example\"\nsymbol = \"wifi\"\naction = \"show-card\"\n",
            &app,
        ).unwrap().remove(0);
        let id = item.id.clone();
        assert!(matches!(
            item.action,
            ItemAction::ShowApplicationCard { .. }
        ));
        let mut bar = bar(Rc::new(Cell::new(0)));
        bar.entries.push(StatusEntry {
            item,
            button: Button::new(""),
        });
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        assert_eq!(bar.resolved_frames(bounds).entries.len(), 4);
        bar.preferences
            .borrow_mut()
            .set_placement(&id, StatusItemPlacement::Pinned);
        let frames = bar.resolved_frames(bounds);
        assert_eq!(frames.entries.len(), 5);
        assert_eq!(bar.entries[frames.entries[4].0].item.id, id);
        bar.preferences
            .borrow_mut()
            .set_placement(&id, StatusItemPlacement::Hidden);
        assert_eq!(bar.resolved_frames(bounds).entries.len(), 4);
    }
    #[test]
    fn clicking_the_clock_uses_its_registered_action_only() {
        let count = Rc::new(Cell::new(0));
        let clicked = count.clone();
        let bar = bar(Rc::new(Cell::new(0))).on_clock_click(move || clicked.set(clicked.get() + 1));
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let frame = bar.resolved_frames(bounds).date;
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        click(
            &bar,
            bounds,
            Point::new(frame.origin.x + 20.0, frame.origin.y + 12.0),
            &mut context,
        );
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn registered_icon_dispatch_does_not_toggle_the_background_or_calendar() {
        let controls = State::new(false);
        let notifications = State::new(false);
        let shared = Rc::new(RefCell::new(ControlCenterInteraction::default()));
        let bar = view(
            State::new(SystemBarState::default()),
            State::new(false),
            controls.clone(),
            notifications.clone(),
            State::new(Vec::new()),
            Rc::new(RefCell::new(ControlCenterPreferences::default())),
            shared.clone(),
        );
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let x = 1440.0 - metrics::CHROME_RIGHT - metrics::STATUS_GROUP_WIDTH;
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        click(&bar, bounds, Point::new(x + 50.0, 27.0), &mut context);
        assert!(!controls.get());
        click(&bar, bounds, Point::new(x + 172.0, 27.0), &mut context);
        assert!(notifications.get());
        assert!(!controls.get());
        click(&bar, bounds, Point::new(x + 244.0, 27.0), &mut context);
        assert!(!notifications.get());
        assert!(controls.get());
        assert_eq!(
            shared.borrow().pending_status_item.as_deref(),
            Some("builtin.network")
        );
    }
}
