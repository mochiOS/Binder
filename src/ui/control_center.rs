use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::control_center_preferences::ControlCenterPreferences;
use crate::platform::{AppInfo, DesktopPlatform, NetworkState, SystemAction, SystemBarState};
use crate::window::DesktopWindows;
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    platform::{Key, PointerButton},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const PANEL_WIDTH: f32 = 372.0;
const PANEL_MARGIN: f32 = 10.0;
const PANEL_TOP: f32 = 46.0;
const PANEL_PADDING: f32 = 16.0;
const HEADER_HEIGHT: f32 = 34.0;
const TILE_GAP: f32 = 10.0;
const TILE_HEIGHT: f32 = 76.0;
const MAX_APP_ITEMS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ItemAction {
    ToggleInput,
    OpenSettings,
    Lock,
    OpenApplication(String),
}

#[derive(Clone, Debug)]
struct Item {
    id: String,
    title: String,
    subtitle: String,
    symbol: SymbolName,
    action: ItemAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Hit {
    Edit,
    Item { id: String, hidden: bool },
}

#[derive(Default)]
pub(crate) struct ControlCenterInteraction {
    hovered: Option<Hit>,
    pressed: Option<Hit>,
}

pub(crate) struct ControlCenterLayer<C> {
    content: C,
    open: State<bool>,
    editing: State<bool>,
    system_bar: State<SystemBarState>,
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    interaction: Rc<RefCell<ControlCenterInteraction>>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
}

impl<C: View> ControlCenterLayer<C> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        content: C,
        open: State<bool>,
        editing: State<bool>,
        system_bar: State<SystemBarState>,
        preferences: Rc<RefCell<ControlCenterPreferences>>,
        interaction: Rc<RefCell<ControlCenterInteraction>>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
        fast_poll_until: Rc<Cell<Option<Instant>>>,
    ) -> Self {
        Self {
            content,
            open,
            editing,
            system_bar,
            preferences,
            interaction,
            platform,
            windows,
            apps,
            fast_poll_until,
        }
    }

    fn items(&self) -> Vec<Item> {
        let state = self.system_bar.get();
        let network = match state.network {
            NetworkState::Connected { network_name, .. } => {
                network_name.unwrap_or_else(|| String::from("Connected"))
            }
            NetworkState::Connecting => String::from("Connecting"),
            NetworkState::Disconnected => String::from("Not connected"),
            NetworkState::Unavailable => String::from("Unavailable"),
        };
        let volume = if !state.volume.available {
            String::from("Unavailable")
        } else if state.volume.muted {
            String::from("Muted")
        } else {
            format!("{}%", state.volume.level)
        };
        let mut items = vec![
            Item::builtin(
                "builtin.network",
                "Network",
                network,
                SymbolName::Network,
                ItemAction::OpenSettings,
            ),
            Item::builtin(
                "builtin.volume",
                "Sound",
                volume,
                SymbolName::Volume2,
                ItemAction::OpenSettings,
            ),
            Item::builtin(
                "builtin.appearance",
                "Appearance",
                "Display settings",
                SymbolName::Paintbrush,
                ItemAction::OpenSettings,
            ),
            Item::builtin(
                "builtin.input",
                "Input",
                if state.japanese_input {
                    "Japanese"
                } else {
                    "Roman"
                },
                SymbolName::Keyboard,
                ItemAction::ToggleInput,
            ),
            Item::builtin(
                "builtin.settings",
                "Settings",
                "System settings",
                SymbolName::Settings,
                ItemAction::OpenSettings,
            ),
            Item::builtin(
                "builtin.lock",
                "Lock",
                "Lock this session",
                SymbolName::Lock,
                ItemAction::Lock,
            ),
        ];
        for app in self.apps.get() {
            items.extend(read_app_items(&app));
        }
        items
    }

    fn ordered_items(&self) -> (Vec<Item>, Vec<Item>) {
        let mut items = self.items();
        let ordered = self
            .preferences
            .borrow()
            .ordered_ids(items.iter().map(|item| item.id.as_str()));
        items.sort_by_key(|item| {
            ordered
                .iter()
                .position(|id| id == &item.id)
                .unwrap_or(usize::MAX)
        });
        let preferences = self.preferences.borrow();
        items
            .into_iter()
            .partition(|item| !preferences.is_hidden(&item.id))
    }

    fn panel(&self, bounds: Rect, visible: usize, hidden: usize) -> Rect {
        let visible_rows = visible.div_ceil(2).max(1);
        let hidden_rows = if self.editing.get() && hidden > 0 {
            hidden.div_ceil(2) + 1
        } else {
            0
        };
        let height = PANEL_PADDING * 2.0
            + HEADER_HEIGHT
            + visible_rows as f32 * TILE_HEIGHT
            + visible_rows.saturating_sub(1) as f32 * TILE_GAP
            + hidden_rows as f32 * (TILE_HEIGHT + TILE_GAP);
        Rect::new(
            bounds.origin.x + (bounds.size.width - PANEL_WIDTH - PANEL_MARGIN).max(PANEL_MARGIN),
            bounds.origin.y + PANEL_TOP,
            PANEL_WIDTH.min(bounds.size.width - PANEL_MARGIN * 2.0),
            height.min(bounds.size.height - PANEL_TOP - PANEL_MARGIN),
        )
    }

    fn edit_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width - 70.0,
            panel.origin.y + 12.0,
            54.0,
            28.0,
        )
    }

    fn tile_rect(panel: Rect, index: usize, start_y: f32) -> Rect {
        let width = (panel.size.width - PANEL_PADDING * 2.0 - TILE_GAP) / 2.0;
        Rect::new(
            panel.origin.x + PANEL_PADDING + (index % 2) as f32 * (width + TILE_GAP),
            start_y + (index / 2) as f32 * (TILE_HEIGHT + TILE_GAP),
            width,
            TILE_HEIGHT,
        )
    }

    fn hit(&self, bounds: Rect, position: Point) -> Option<Hit> {
        let (visible, hidden) = self.ordered_items();
        let panel = self.panel(bounds, visible.len(), hidden.len());
        if !panel.contains(position) {
            return None;
        }
        if Self::edit_rect(panel).contains(position) {
            return Some(Hit::Edit);
        }
        let start_y = panel.origin.y + PANEL_PADDING + HEADER_HEIGHT;
        for (index, item) in visible.iter().enumerate() {
            if Self::tile_rect(panel, index, start_y).contains(position) {
                return Some(Hit::Item {
                    id: item.id.clone(),
                    hidden: false,
                });
            }
        }
        if self.editing.get() {
            let visible_rows = visible.len().div_ceil(2).max(1);
            let hidden_y = start_y + visible_rows as f32 * (TILE_HEIGHT + TILE_GAP) + 24.0;
            for (index, item) in hidden.iter().enumerate() {
                if Self::tile_rect(panel, index, hidden_y).contains(position) {
                    return Some(Hit::Item {
                        id: item.id.clone(),
                        hidden: true,
                    });
                }
            }
        }
        None
    }

    fn save_preferences(&self) {
        if let Err(error) = self.preferences.borrow().save() {
            eprintln!("failed to save Control Center preferences: {error}");
        }
    }

    fn activate(&self, item: &Item) {
        match &item.action {
            ItemAction::ToggleInput => {
                let enabled = viewkit::platform::input_method::toggle()
                    .unwrap_or(!self.system_bar.get().japanese_input);
                self.system_bar
                    .update_if_changed(|state| state.japanese_input = enabled);
            }
            ItemAction::OpenSettings => self.open_application("org.mochios.settings"),
            ItemAction::Lock => {
                if let Err(error) = self
                    .platform
                    .borrow()
                    .perform_system_action(SystemAction::LockScreen)
                {
                    eprintln!("failed to lock session: {error:?}");
                }
            }
            ItemAction::OpenApplication(bundle_id) => self.open_application(bundle_id),
        }
    }

    fn open_application(&self, bundle_id: &str) {
        if let Some(process) = self.platform.borrow().process_id_for_bundle(bundle_id) {
            if let Err(error) = self.platform.borrow().activate_application(process) {
                eprintln!("failed to activate {bundle_id}: {error:?}");
            }
            self.windows
                .update(|windows| windows.activate_process(process));
            return;
        }
        let Some(app) = self
            .apps
            .get()
            .into_iter()
            .find(|app| app.bundle_id == bundle_id)
        else {
            eprintln!("Control Center application is not installed: {bundle_id}");
            return;
        };
        match self.platform.borrow_mut().launch_app(&app) {
            Ok(_) => self
                .fast_poll_until
                .set(Some(Instant::now() + Duration::from_secs(5))),
            Err(error) => eprintln!("failed to launch {bundle_id}: {error:?}"),
        }
    }

    fn paint_tile(&self, item: &Item, bounds: Rect, hidden: bool, context: &mut PaintContext<'_>) {
        let hit = Hit::Item {
            id: item.id.clone(),
            hidden,
        };
        let hovered = self.interaction.borrow().hovered.as_ref() == Some(&hit);
        Rectangle::new()
            .color(RectangleColor::Custom(if hovered {
                Theme::current().shell.item_hover
            } else {
                Theme::current().shell.item_enabled
            }))
            .radius(CornerRadius::Custom(16.0))
            .paint(bounds, context);
        let icon = Rect::new(bounds.origin.x + 14.0, bounds.origin.y + 18.0, 40.0, 40.0);
        Rectangle::new()
            .color(RectangleColor::Custom(Theme::current().colors.accent))
            .radius(CornerRadius::Custom(20.0))
            .paint(icon, context);
        Icon::new(item.symbol)
            .size(20.0)
            .color(Color::WHITE)
            .paint(icon, context);
        Text::styled(item.title.clone(), TextRole::Body)
            .weight(650)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    bounds.origin.x + 64.0,
                    bounds.origin.y + 15.0,
                    bounds.size.width - 74.0,
                    22.0,
                ),
                context,
            );
        Text::styled(item.subtitle.clone(), TextRole::Caption)
            .color(Theme::current().shell.secondary_text)
            .paint(
                Rect::new(
                    bounds.origin.x + 64.0,
                    bounds.origin.y + 39.0,
                    bounds.size.width - 74.0,
                    20.0,
                ),
                context,
            );
        if self.editing.get() {
            let badge = Rect::new(bounds.origin.x - 5.0, bounds.origin.y - 5.0, 24.0, 24.0);
            Rectangle::new()
                .color(RectangleColor::Custom(if hidden {
                    Theme::current().colors.accent
                } else {
                    Theme::current().shell.alert
                }))
                .radius(CornerRadius::Custom(12.0))
                .paint(badge, context);
            Icon::new(if hidden {
                SymbolName::Plus
            } else {
                SymbolName::Minus
            })
            .size(13.0)
            .color(Color::WHITE)
            .paint(badge, context);
        }
    }
}

impl Item {
    fn builtin(
        id: &str,
        title: &str,
        subtitle: impl Into<String>,
        symbol: SymbolName,
        action: ItemAction,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.into(),
            symbol,
            action,
        }
    }
}

impl<C: View> View for ControlCenterLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        if !self.open.get() {
            return;
        }
        let (visible, hidden) = self.ordered_items();
        let panel = self.panel(bounds, visible.len(), hidden.len());
        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.panel_background,
            ))
            .radius(CornerRadius::Custom(22.0))
            .shadow(ShadowStyle::Card)
            .border(BorderStyle::custom(
                Theme::current().shell.panel_border,
                1.0,
            ))
            .paint(panel, context);
        Text::styled("Control Center", TextRole::TitleSmall)
            .weight(700)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    panel.origin.x + PANEL_PADDING,
                    panel.origin.y + 13.0,
                    220.0,
                    26.0,
                ),
                context,
            );
        let edit = Self::edit_rect(panel);
        if self.interaction.borrow().hovered == Some(Hit::Edit) {
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_hover))
                .radius(CornerRadius::Custom(14.0))
                .paint(edit, context);
        }
        Text::styled(
            if self.editing.get() { "Done" } else { "Edit" },
            TextRole::Caption,
        )
        .weight(650)
        .alignment(TextAlignment::Center)
        .color(Theme::current().colors.accent)
        .paint(edit, context);

        let start_y = panel.origin.y + PANEL_PADDING + HEADER_HEIGHT;
        for (index, item) in visible.iter().enumerate() {
            self.paint_tile(item, Self::tile_rect(panel, index, start_y), false, context);
        }
        if self.editing.get() && !hidden.is_empty() {
            let visible_rows = visible.len().div_ceil(2).max(1);
            let label_y = start_y + visible_rows as f32 * (TILE_HEIGHT + TILE_GAP);
            Text::styled("MORE CONTROLS", TextRole::Caption)
                .weight(650)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(panel.origin.x + PANEL_PADDING, label_y, 180.0, 20.0),
                    context,
                );
            for (index, item) in hidden.iter().enumerate() {
                self.paint_tile(
                    item,
                    Self::tile_rect(panel, index, label_y + 24.0),
                    true,
                    context,
                );
            }
        }
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
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
                self.editing.set(false);
                self.interaction.borrow_mut().hovered = None;
                context.request_redraw();
            }
            ViewEvent::PointerMoved { position } => {
                let hit = self.hit(bounds, *position);
                if self.interaction.borrow().hovered != hit {
                    self.interaction.borrow_mut().hovered = hit;
                    context.request_redraw();
                }
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                let hit = self.hit(bounds, *position);
                if hit.is_none() {
                    self.open.set(false);
                    self.editing.set(false);
                }
                self.interaction.borrow_mut().pressed = hit;
                context.request_redraw();
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let released = self.hit(bounds, *position);
                let pressed = self.interaction.borrow_mut().pressed.take();
                match (pressed, released) {
                    (Some(Hit::Edit), Some(Hit::Edit)) => self.editing.set(!self.editing.get()),
                    (
                        Some(Hit::Item {
                            id: from,
                            hidden: false,
                        }),
                        Some(Hit::Item {
                            id: to,
                            hidden: false,
                        }),
                    ) if self.editing.get() && from != to => {
                        let ids = self
                            .items()
                            .into_iter()
                            .map(|item| item.id)
                            .collect::<Vec<_>>();
                        if self.preferences.borrow_mut().move_before(&from, &to, &ids) {
                            self.save_preferences();
                        }
                    }
                    (
                        Some(Hit::Item { id, hidden }),
                        Some(Hit::Item {
                            id: released,
                            hidden: released_hidden,
                        }),
                    ) if self.editing.get() && id == released && hidden == released_hidden => {
                        self.preferences.borrow_mut().set_hidden(&id, !hidden);
                        self.save_preferences();
                    }
                    (
                        Some(Hit::Item { id, hidden: false }),
                        Some(Hit::Item {
                            id: released,
                            hidden: false,
                        }),
                    ) if id == released => {
                        if let Some(item) = self.items().into_iter().find(|item| item.id == id) {
                            self.activate(&item);
                        }
                        self.open.set(false);
                    }
                    _ => {}
                }
                context.request_redraw();
            }
            _ => {}
        }
        EventResult::Consumed
    }
}

fn read_app_items(app: &AppInfo) -> Vec<Item> {
    let Ok(text) = fs::read_to_string(app.root.join("control-center.toml")) else {
        return Vec::new();
    };
    parse_app_items(&text, app).unwrap_or_else(|error| {
        eprintln!(
            "ignored invalid Control Center registration for {}: {error}",
            app.bundle_id
        );
        Vec::new()
    })
}

fn parse_app_items(text: &str, app: &AppInfo) -> Result<Vec<Item>, &'static str> {
    let valid_format = text.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(key, value)| key.trim() == "format" && value.trim() == "1")
    });
    if !valid_format {
        return Err("unsupported format");
    }
    let mut result = Vec::new();
    for section in text.split("[[item]]").skip(1).take(MAX_APP_ITEMS + 1) {
        if result.len() == MAX_APP_ITEMS {
            return Err("too many items");
        }
        let field = |name: &str| -> Option<&str> {
            section.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                if key.trim() != name {
                    return None;
                }
                value.trim().strip_prefix('"')?.strip_suffix('"')
            })
        };
        let id = field("id").ok_or("missing id")?;
        let title = field("title").ok_or("missing title")?;
        let symbol = field("symbol")
            .and_then(SymbolName::from_asset_name)
            .ok_or("invalid symbol")?;
        if !valid_local_id(id)
            || title.is_empty()
            || title.len() > 40
            || field("action") != Some("open-application")
        {
            return Err("invalid item fields");
        }
        result.push(Item {
            id: format!("app.{}:{id}", app.bundle_id),
            title: title.to_string(),
            subtitle: app.name.clone(),
            symbol,
            action: ItemAction::OpenApplication(app.bundle_id.clone()),
        });
    }
    Ok(result)
}

fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app() -> AppInfo {
        AppInfo {
            root: PathBuf::from("/test/Edit.app"),
            name: String::from("Edit"),
            bundle_id: String::from("org.mochios.edit"),
            version: String::from("1"),
            developer: String::from("mochiOS"),
            entry: String::from("entry.elf"),
            description: String::new(),
            icon: None,
            resources: Vec::new(),
        }
    }

    #[test]
    fn parses_safe_application_registration() {
        let items = parse_app_items(
            "format = 1\n[[item]]\nid = \"new-document\"\ntitle = \"New document\"\nsymbol = \"pencil\"\naction = \"open-application\"\n",
            &app(),
        ).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "app.org.mochios.edit:new-document");
        assert_eq!(
            items[0].action,
            ItemAction::OpenApplication(String::from("org.mochios.edit"))
        );
    }

    #[test]
    fn rejects_arbitrary_actions_and_symbols() {
        assert!(
            parse_app_items(
                "format=1\n[[item]]\nid=\"x\"\ntitle=\"X\"\nsymbol=\"missing\"\naction=\"shell\"",
                &app()
            )
            .is_err()
        );
        assert!(
            parse_app_items(
                "format=2\n[[item]]\nid=\"x\"\ntitle=\"X\"\nsymbol=\"pencil\"\naction=\"open-application\"",
                &app()
            )
            .is_err()
        );
    }
}
