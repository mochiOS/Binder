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

const PANEL_WIDTH: f32 = 300.0;
const PANEL_MARGIN: f32 = 10.0;
const PANEL_TOP: f32 = 46.0;
const PANEL_PADDING: f32 = 16.0;
const HEADER_HEIGHT: f32 = 34.0;
const VOLUME_ROW_HEIGHT: f32 = 52.0;
const VOLUME_ROW_GAP: f32 = 12.0;
const TILE_COLUMNS: usize = 4;
const TILE_GAP: f32 = 12.0;
const TILE_HEIGHT: f32 = 56.0;
const CONTROL_SIZE: f32 = 52.0;
const ICON_SIZE: f32 = 20.0;
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
    symbol: SymbolName,
    action: ItemAction,
    is_on: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CustomizationItem {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) source: String,
    pub(crate) symbol: SymbolName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Hit {
    Edit,
    Mute,
    Item { id: String },
}

#[derive(Default)]
struct AppItemCache {
    apps: Vec<AppInfo>,
    items: Vec<Item>,
}

#[derive(Default)]
pub(crate) struct ControlCenterInteraction {
    hovered: Option<Hit>,
    pressed: Option<Hit>,
}

pub(crate) struct ControlCenterLayer<C> {
    content: C,
    open: State<bool>,
    system_bar: State<SystemBarState>,
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    interaction: Rc<RefCell<ControlCenterInteraction>>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
    app_item_cache: RefCell<AppItemCache>,
    volume_level: State<f32>,
    volume_interaction: SliderInteractionState,
}

impl<C: View> ControlCenterLayer<C> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        content: C,
        open: State<bool>,
        system_bar: State<SystemBarState>,
        preferences: Rc<RefCell<ControlCenterPreferences>>,
        interaction: Rc<RefCell<ControlCenterInteraction>>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
        fast_poll_until: Rc<Cell<Option<Instant>>>,
    ) -> Self {
        let volume_level = State::new(f32::from(system_bar.get().volume.level));
        Self {
            content,
            open,
            system_bar,
            preferences,
            interaction,
            platform,
            windows,
            apps,
            fast_poll_until,
            app_item_cache: RefCell::new(AppItemCache::default()),
            volume_level,
            volume_interaction: SliderInteractionState::new(),
        }
    }

    fn items(&self) -> Vec<Item> {
        let state = self.system_bar.get();
        let network_on = matches!(
            &state.network,
            NetworkState::Connected { .. } | NetworkState::Connecting
        );
        let mut items = vec![
            Item::builtin(
                "builtin.network",
                "Network",
                SymbolName::Network,
                ItemAction::OpenSettings,
                network_on,
            ),
            Item::builtin(
                "builtin.appearance",
                "Appearance",
                SymbolName::Paintbrush,
                ItemAction::OpenSettings,
                true,
            ),
            Item::builtin(
                "builtin.input",
                "Input",
                SymbolName::Keyboard,
                ItemAction::ToggleInput,
                state.japanese_input,
            ),
            Item::builtin(
                "builtin.settings",
                "Settings",
                SymbolName::Settings,
                ItemAction::OpenSettings,
                true,
            ),
            Item::builtin(
                "builtin.lock",
                "Lock",
                SymbolName::Lock,
                ItemAction::Lock,
                true,
            ),
        ];
        items.extend(self.cached_app_items());
        items
    }

    fn cached_app_items(&self) -> Vec<Item> {
        self.apps.with(|apps| {
            let mut cache = self.app_item_cache.borrow_mut();
            if cache.apps != *apps {
                cache.items = apps.iter().flat_map(read_app_items).collect();
                cache.apps.clone_from(apps);
            }
            cache.items.clone()
        })
    }

    fn ordered_items(&self) -> Vec<Item> {
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
            .filter(|item| !preferences.is_hidden(&item.id))
            .collect()
    }

    fn panel(&self, bounds: Rect, visible: usize) -> Rect {
        let visible_rows = visible.div_ceil(TILE_COLUMNS).max(1);
        let height = PANEL_PADDING * 2.0
            + HEADER_HEIGHT
            + VOLUME_ROW_HEIGHT
            + VOLUME_ROW_GAP
            + visible_rows as f32 * TILE_HEIGHT
            + visible_rows.saturating_sub(1) as f32 * TILE_GAP;
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
        let width = (panel.size.width - PANEL_PADDING * 2.0 - TILE_GAP * (TILE_COLUMNS - 1) as f32)
            / TILE_COLUMNS as f32;
        Rect::new(
            panel.origin.x + PANEL_PADDING + (index % TILE_COLUMNS) as f32 * (width + TILE_GAP),
            start_y + (index / TILE_COLUMNS) as f32 * (TILE_HEIGHT + TILE_GAP),
            width,
            TILE_HEIGHT,
        )
    }

    fn volume_row_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + PANEL_PADDING + HEADER_HEIGHT,
            panel.size.width - PANEL_PADDING * 2.0,
            VOLUME_ROW_HEIGHT,
        )
    }

    fn mute_rect(panel: Rect) -> Rect {
        let row = Self::volume_row_rect(panel);
        Rect::new(row.origin.x + 6.0, row.origin.y + 6.0, 40.0, 40.0)
    }

    fn volume_slider_rect(panel: Rect) -> Rect {
        let row = Self::volume_row_rect(panel);
        Rect::new(
            row.origin.x + 56.0,
            row.origin.y + 8.0,
            row.size.width - 68.0,
            row.size.height - 16.0,
        )
    }

    fn tile_start_y(panel: Rect) -> f32 {
        let row = Self::volume_row_rect(panel);
        row.origin.y + row.size.height + VOLUME_ROW_GAP
    }

    fn volume_slider(&self) -> Slider {
        Slider::with_interaction(self.volume_level.binding(), self.volume_interaction.clone())
            .range(0.0..=100.0)
            .step(1.0)
            .enabled(self.system_bar.get().volume.available)
    }

    fn hit(&self, bounds: Rect, position: Point) -> Option<Hit> {
        let visible = self.ordered_items();
        let panel = self.panel(bounds, visible.len());
        if !panel.contains(position) {
            return None;
        }
        if Self::edit_rect(panel).contains(position) {
            return Some(Hit::Edit);
        }
        if Self::mute_rect(panel).contains(position) {
            return Some(Hit::Mute);
        }
        let start_y = Self::tile_start_y(panel);
        for (index, item) in visible.iter().enumerate() {
            if Self::tile_rect(panel, index, start_y).contains(position) {
                return Some(Hit::Item {
                    id: item.id.clone(),
                });
            }
        }
        None
    }

    fn save_preferences(&self) {
        if let Err(error) = self.preferences.borrow().save() {
            eprintln!("failed to save Control Center preferences: {error}");
        }
    }

    fn open_customizer(&self) {
        let available = customization_items(&self.apps.get())
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        if self.preferences.borrow_mut().reconcile(&available) {
            self.save_preferences();
        }
        self.windows
            .update(|windows| windows.open_control_center_editor());
        self.open.set(false);
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

    fn set_volume(&self, level: f32) {
        let level = level.round().clamp(0.0, 100.0) as u8;
        match self.platform.borrow_mut().set_output_volume(level) {
            Ok(volume) => {
                self.volume_level.set_if_changed(f32::from(volume.level));
                self.system_bar
                    .update_if_changed(|state| state.volume = volume);
            }
            Err(error) => eprintln!("failed to set output volume: {error:?}"),
        }
    }

    fn toggle_mute(&self) {
        let muted = !self.system_bar.get().volume.muted;
        match self.platform.borrow_mut().set_output_muted(muted) {
            Ok(volume) => {
                self.volume_level.set_if_changed(f32::from(volume.level));
                self.system_bar
                    .update_if_changed(|state| state.volume = volume);
            }
            Err(error) => eprintln!("failed to set output mute: {error:?}"),
        }
    }

    fn paint_volume(&self, panel: Rect, context: &mut PaintContext<'_>) {
        let row = Self::volume_row_rect(panel);
        Rectangle::new()
            .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
            .radius(CornerRadius::Custom(16.0))
            .paint(row, context);

        let volume = self.system_bar.get().volume;
        if !self.volume_interaction.is_dragging() {
            self.volume_level.set_if_changed(f32::from(volume.level));
        }
        let mute = Self::mute_rect(panel);
        let muted = !volume.available || volume.muted || volume.level == 0;
        Rectangle::new()
            .color(RectangleColor::Custom(if muted {
                Theme::current().shell.control_hover
            } else {
                Theme::current().colors.accent
            }))
            .radius(CornerRadius::Custom(20.0))
            .paint(mute, context);
        Icon::new(if muted {
            SymbolName::VolumeMute
        } else {
            SymbolName::VolumeHigh
        })
        .size(18.0)
        .color(if muted {
            Theme::current().shell.primary_text
        } else {
            Color::WHITE
        })
        .accessibility_label(if muted { "Unmute" } else { "Mute" })
        .paint(mute, context);
        self.volume_slider()
            .paint(Self::volume_slider_rect(panel), context);
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

    fn paint_tile(&self, item: &Item, bounds: Rect, context: &mut PaintContext<'_>) {
        let hit = Hit::Item {
            id: item.id.clone(),
        };
        let hovered = self.interaction.borrow().hovered.as_ref() == Some(&hit);
        let control = Rect::new(
            bounds.origin.x + (bounds.size.width - CONTROL_SIZE) / 2.0,
            bounds.origin.y + (bounds.size.height - CONTROL_SIZE) / 2.0,
            CONTROL_SIZE,
            CONTROL_SIZE,
        );
        let is_on = item.is_on;
        Rectangle::new()
            .color(RectangleColor::Custom(if is_on {
                if hovered {
                    Theme::current().shell.action_hover
                } else {
                    Theme::current().colors.accent
                }
            } else if hovered {
                Theme::current().shell.control_hover
            } else {
                Theme::current().shell.item_enabled
            }))
            .radius(CornerRadius::Custom(CONTROL_SIZE / 2.0))
            .paint(control, context);
        Icon::new(item.symbol)
            .size(ICON_SIZE)
            .color(if is_on {
                Color::WHITE
            } else {
                Theme::current().shell.primary_text
            })
            .accessibility_label(item.title.clone())
            .paint(control, context);
    }
}

impl Item {
    fn builtin(id: &str, title: &str, symbol: SymbolName, action: ItemAction, is_on: bool) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            symbol,
            action,
            is_on,
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
        let visible = self.ordered_items();
        let panel = self.panel(bounds, visible.len());
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
        Text::styled("Edit", TextRole::Caption)
            .weight(650)
            .alignment(TextAlignment::Center)
            .color(Theme::current().colors.accent)
            .paint(edit, context);

        self.paint_volume(panel, context);

        let start_y = Self::tile_start_y(panel);
        for (index, item) in visible.iter().enumerate() {
            self.paint_tile(item, Self::tile_rect(panel, index, start_y), context);
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
        let visible = self.ordered_items();
        let panel = self.panel(bounds, visible.len());
        let before_volume = self.volume_level.get();
        let slider_result =
            self.volume_slider()
                .handle_event(Self::volume_slider_rect(panel), event, context);
        let after_volume = self.volume_level.get();
        if (before_volume - after_volume).abs() >= 0.5 {
            self.set_volume(after_volume);
        }
        if slider_result == EventResult::Consumed {
            return EventResult::Consumed;
        }
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
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
                    (Some(Hit::Edit), Some(Hit::Edit)) => self.open_customizer(),
                    (Some(Hit::Mute), Some(Hit::Mute)) => self.toggle_mute(),
                    (Some(Hit::Item { id }), Some(Hit::Item { id: released }))
                        if id == released =>
                    {
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
    let Ok(text) = fs::read_to_string(app.root.join("manifest.toml")) else {
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

pub(crate) fn customization_items(apps: &[AppInfo]) -> Vec<CustomizationItem> {
    let mut items = vec![
        CustomizationItem {
            id: String::from("builtin.network"),
            title: String::from("Network"),
            source: String::from("System"),
            symbol: SymbolName::Network,
        },
        CustomizationItem {
            id: String::from("builtin.appearance"),
            title: String::from("Appearance"),
            source: String::from("System"),
            symbol: SymbolName::Paintbrush,
        },
        CustomizationItem {
            id: String::from("builtin.input"),
            title: String::from("Input"),
            source: String::from("System"),
            symbol: SymbolName::Keyboard,
        },
        CustomizationItem {
            id: String::from("builtin.settings"),
            title: String::from("Settings"),
            source: String::from("System"),
            symbol: SymbolName::Settings,
        },
        CustomizationItem {
            id: String::from("builtin.lock"),
            title: String::from("Lock"),
            source: String::from("System"),
            symbol: SymbolName::Lock,
        },
    ];
    items.extend(apps.iter().flat_map(|app| {
        read_app_items(app)
            .into_iter()
            .map(|item| CustomizationItem {
                id: item.id,
                title: item.title,
                source: app.name.clone(),
                symbol: item.symbol,
            })
    }));
    items
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
    for section in text
        .split("[[application.control_center_items]]")
        .skip(1)
        .take(MAX_APP_ITEMS + 1)
    {
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
            symbol,
            action: ItemAction::OpenApplication(app.bundle_id.clone()),
            is_on: true,
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
            "format = 1\n[[application.control_center_items]]\nid = \"new-document\"\ntitle = \"New document\"\nsymbol = \"pencil\"\naction = \"open-application\"\n",
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
                "format=1\n[[application.control_center_items]]\nid=\"x\"\ntitle=\"X\"\nsymbol=\"missing\"\naction=\"shell\"",
                &app()
            )
            .is_err()
        );
        assert!(
            parse_app_items(
                "format=2\n[[application.control_center_items]]\nid=\"x\"\ntitle=\"X\"\nsymbol=\"pencil\"\naction=\"open-application\"",
                &app()
            )
            .is_err()
        );
    }
}
