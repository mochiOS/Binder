use super::desktop_metrics as metrics;
use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::control_center_preferences::ControlCenterPreferences;
use crate::platform::{AppInfo, DesktopPlatform, NetworkState, SystemAction, SystemBarState};
use crate::window::DesktopWindows;
use viewkit::{
    animation::VisibilityTransition,
    draw_command::PaintTransform,
    event::{EventContext, EventResult, ViewEvent},
    platform::{Key, PointerButton},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const PANEL_WIDTH: f32 = 300.0;
const PANEL_MARGIN: f32 = metrics::CHROME_RIGHT;
const PANEL_TOP: f32 = metrics::CONTROL_TOP;
const PANEL_PADDING: f32 = 16.0;
const MAX_APP_ITEMS: usize = 8;
const TILE_LABEL_WIDTH: f32 = 135.0;
const TILE_LABEL_GAP: f32 = 10.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ItemAction {
    ToggleInput,
    ToggleFocus,
    ShowNetwork,
    OpenSettings,
    Lock,
    OpenApplication(String),
    ShowApplicationCard { bundle_id: String, item_id: String },
}

#[derive(Clone, Debug)]
struct Item {
    id: String,
    title: String,
    symbol: SymbolName,
    action: ItemAction,
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
    Back,
    Mute,
    Item { id: String },
}

#[derive(Default)]
struct AppItemCache {
    apps: Vec<AppInfo>,
    items: Vec<Item>,
}

/// Grid/detail share a reversible transition; retain the outgoing detail until its last frame.
#[derive(Default)]
struct DetailNavigation {
    transition: VisibilityTransition,
    detail: Option<ControlCenterDetail>,
}

impl DetailNavigation {
    fn sample(
        &mut self,
        target: &Option<ControlCenterDetail>,
        motion: viewkit::theme::Motion,
        now: Instant,
    ) -> (f32, Option<Instant>) {
        if target.is_some() {
            self.detail = target.clone();
        }
        let result = self.transition.sample(target.is_some(), motion, now);
        if result == (0.0, None) {
            self.detail = None;
        }
        result
    }
}

fn navigation_transform(amount: f32, detail: bool, distance: f32) -> PaintTransform {
    PaintTransform {
        anchor: Point::new(0.0, 0.0),
        scale: 1.0,
        offset: Point::new(
            if detail {
                distance * (1.0 - amount)
            } else {
                -distance * amount
            },
            0.0,
        ),
        opacity: if detail { amount } else { 1.0 - amount },
    }
}

pub(crate) struct ControlCenterInteraction {
    transition: VisibilityTransition,
    navigation: DetailNavigation,
    navigation_presented: Option<(bool, PaintTransform)>,
    presented: Option<PaintTransform>,
    closing_detail: Option<ControlCenterDetail>,
    damage: Option<Rect>,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    focused: Option<Hit>,
    volume_level: State<f32>,
    volume_interaction: SliderInteractionState,
    detail: State<Option<ControlCenterDetail>>,
    network_enabled: State<bool>,
}
impl Default for ControlCenterInteraction {
    fn default() -> Self {
        Self {
            transition: VisibilityTransition::default(),
            navigation: DetailNavigation::default(),
            navigation_presented: None,
            presented: None,
            closing_detail: None,
            damage: None,
            hovered: None,
            pressed: None,
            focused: None,
            volume_level: State::new(0.0),
            volume_interaction: SliderInteractionState::new(),
            detail: State::new(None),
            network_enabled: State::new(false),
        }
    }
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
    detail: State<Option<ControlCenterDetail>>,
    network_enabled: State<bool>,
    network_switch: Switch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ControlCenterDetail {
    Network,
    Application {
        bundle_id: String,
        item_id: String,
        fallback_title: String,
    },
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
        let saved = interaction.borrow();
        let volume_level = saved.volume_level.clone();
        let volume_interaction = saved.volume_interaction.clone();
        let detail = saved.detail.clone();
        let network_enabled = saved.network_enabled.clone();
        drop(saved);
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
            volume_interaction,
            detail,
            network_enabled: network_enabled.clone(),
            network_switch: Switch::new(network_enabled.binding()),
        }
    }

    fn items(&self) -> Vec<Item> {
        self.cached_app_items()
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
        Self::panel_for_detail(bounds, visible, self.detail.get().is_some())
    }

    fn panel_for_detail(bounds: Rect, visible: usize, detail: bool) -> Rect {
        if detail {
            return Rect::new(
                bounds.origin.x
                    + (bounds.size.width - PANEL_WIDTH - PANEL_MARGIN).max(PANEL_MARGIN),
                bounds.origin.y + PANEL_TOP,
                PANEL_WIDTH.min(bounds.size.width - PANEL_MARGIN * 2.0),
                330.0_f32.min(bounds.size.height - PANEL_TOP - PANEL_MARGIN),
            );
        }
        let height = metrics::CONTROL_WIDTH
            + visible.saturating_sub(5).div_ceil(3) as f32 * metrics::CONTROL_PITCH;
        Rect::new(
            bounds.origin.x
                + (bounds.size.width - metrics::CONTROL_WIDTH - metrics::CHROME_RIGHT).max(0.0),
            bounds.origin.y + metrics::CONTROL_TOP,
            metrics::CONTROL_WIDTH.min(bounds.size.width),
            height,
        )
    }

    fn back_rect(panel: Rect) -> Rect {
        Rect::new(panel.origin.x + 10.0, panel.origin.y + 10.0, 32.0, 30.0)
    }

    fn network_switch_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width - 78.0,
            panel.origin.y + 59.0,
            58.0,
            40.0,
        )
    }

    fn tile_rect(panel: Rect, index: usize, _start_y: f32) -> Rect {
        let (column, row) = if index < 2 {
            (2, index)
        } else {
            ((index - 2) % 3, 2 + (index - 2) / 3)
        };
        Rect::new(
            panel.origin.x + column as f32 * metrics::CONTROL_PITCH,
            panel.origin.y + row as f32 * metrics::CONTROL_PITCH,
            metrics::CONTROL_SIZE,
            metrics::CONTROL_SIZE,
        )
    }
    fn volume_slider_rect(panel: Rect) -> Rect {
        Rect::new(panel.origin.x + 70.0, panel.origin.y, 60.0, 130.0)
    }
    fn brightness_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x,
            panel.origin.y,
            metrics::CONTROL_SIZE,
            metrics::CONTROL_SLIDER_HEIGHT,
        )
    }
    fn tile_start_y(panel: Rect) -> f32 {
        panel.origin.y
    }
    fn volume_slider(&self) -> Slider {
        Slider::with_interaction(self.volume_level.binding(), self.volume_interaction.clone())
            .style(SliderStyle::VerticalLevel)
            .label("Volume")
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
        if self.detail.get().is_some() {
            return Self::back_rect(panel)
                .contains(position)
                .then_some(Hit::Back);
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
            ItemAction::ToggleFocus => {
                let enabled = !self.system_bar.get().notifications.focus_enabled;
                match self
                    .platform
                    .borrow_mut()
                    .set_notification_focus_enabled(enabled)
                {
                    Ok(settings) => {
                        self.system_bar.update_if_changed(|state| {
                            state.notifications.focus_enabled = settings.focus_enabled;
                        });
                    }
                    Err(error) => eprintln!("failed to change Focus mode: {error:?}"),
                }
            }
            ItemAction::ShowNetwork => {
                self.network_enabled
                    .set_if_changed(self.system_bar.get().network.enabled());
                self.detail.set(Some(ControlCenterDetail::Network));
            }
            ItemAction::ShowApplicationCard { bundle_id, item_id } => {
                let registered = self
                    .platform
                    .borrow()
                    .control_center_cards()
                    .into_iter()
                    .any(|card| card.bundle_id == *bundle_id && card.item_id == *item_id);
                if !registered {
                    self.open_application(bundle_id);
                }
                self.detail.set(Some(ControlCenterDetail::Application {
                    bundle_id: bundle_id.clone(),
                    item_id: item_id.clone(),
                    fallback_title: item.title.clone(),
                }));
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

    fn set_network_enabled(&self, enabled: bool) {
        match self.platform.borrow_mut().set_network_enabled(enabled) {
            Ok(network) => {
                self.network_enabled.set_if_changed(network.enabled());
                self.system_bar
                    .update_if_changed(|state| state.network = network);
            }
            Err(error) => {
                self.network_enabled
                    .set_if_changed(self.system_bar.get().network.enabled());
                eprintln!("failed to change network state: {error:?}");
            }
        }
    }

    fn paint_volume(&self, panel: Rect, context: &mut PaintContext<'_>) {
        let volume = self.system_bar.get().volume;
        if !self.volume_interaction.is_dragging() {
            self.volume_level.set_if_changed(f32::from(volume.level));
        }
        let brightness = State::new(0.62);
        Slider::new(brightness.binding())
            .style(SliderStyle::VerticalLevel)
            .label("Brightness — unavailable")
            .enabled(false)
            .paint(Self::brightness_rect(panel), context);
        self.volume_slider()
            .paint(Self::volume_slider_rect(panel), context);
        let frame = Self::volume_slider_rect(panel);
        Icon::new(if volume.muted || volume.level == 0 {
            SymbolName::VolumeMute
        } else {
            SymbolName::VolumeHigh
        })
        .size(22.0)
        .color(Color::rgb(90, 90, 90))
        .accessibility_label("Volume")
        .paint(
            Rect::new(frame.origin.x + 17.0, frame.origin.y + 94.0, 26.0, 26.0),
            context,
        );
    }

    fn paint_network_detail(&self, panel: Rect, context: &mut PaintContext<'_>) {
        let state = self.system_bar.get().network;
        if !self.network_switch.interaction().is_pressed() {
            self.network_enabled.set_if_changed(state.enabled());
        }

        let back = Self::back_rect(panel);
        let mut back_node = viewkit::accessibility::AccessibilityNode::new(
            viewkit::accessibility::AccessibilityRole::Button,
            back,
        );
        back_node.label = Some("Back".into());
        back_node.focusable = true;
        back_node.focused = self.interaction.borrow().focused == Some(Hit::Back);
        context.record_accessibility(back_node);
        if self.interaction.borrow().hovered == Some(Hit::Back) {
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_hover))
                .radius(CornerRadius::Medium)
                .paint(back, context);
        }
        Icon::new(SymbolName::ChevronLeft)
            .size(12.0)
            .color(Theme::current().shell.primary_text)
            .accessibility_label("Back")
            .paint(back, context);
        Text::styled("Network", TextRole::TitleSmall)
            .weight(700)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    panel.origin.x + 44.0,
                    panel.origin.y + 12.0,
                    panel.size.width - 88.0,
                    28.0,
                ),
                context,
            );

        let summary = Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + 50.0,
            panel.size.width - PANEL_PADDING * 2.0,
            62.0,
        );
        Rectangle::new()
            .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
            .radius(CornerRadius::Large)
            .paint(summary, context);
        let icon = Rect::new(summary.origin.x + 10.0, summary.origin.y + 11.0, 40.0, 40.0);
        Rectangle::new()
            .color(RectangleColor::Custom(if state.enabled() {
                Theme::current().colors.accent
            } else {
                Theme::current().shell.control_hover
            }))
            .radius(CornerRadius::Medium)
            .paint(icon, context);
        Icon::new(SymbolName::Network)
            .size(18.0)
            .color(if state.enabled() {
                Color::WHITE
            } else {
                Theme::current().shell.primary_text
            })
            .paint(icon, context);
        let (status, network_name, interface, address) = match &state {
            NetworkState::Unavailable => ("Unavailable", "—", "—", "—"),
            NetworkState::Disabled => ("Off", "—", "—", "—"),
            NetworkState::Disconnected => ("Not Connected", "—", "—", "—"),
            NetworkState::Connecting => ("Connecting…", "—", "—", "—"),
            NetworkState::Connected {
                network_name,
                interface,
                address,
                ..
            } => (
                "Connected",
                network_name.as_deref().unwrap_or("Connected network"),
                interface.as_deref().unwrap_or("—"),
                address.as_deref().unwrap_or("—"),
            ),
        };
        Text::styled(status, TextRole::Label)
            .weight(650)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    summary.origin.x + 60.0,
                    summary.origin.y + 11.0,
                    126.0,
                    22.0,
                ),
                context,
            );
        Text::styled(
            if state.available() {
                "Network"
            } else {
                "No service"
            },
            TextRole::Caption,
        )
        .color(Theme::current().shell.secondary_text)
        .paint(
            Rect::new(
                summary.origin.x + 60.0,
                summary.origin.y + 33.0,
                126.0,
                18.0,
            ),
            context,
        );
        self.network_switch
            .paint(Self::network_switch_rect(panel), context);

        let details = [
            ("Network", network_name),
            ("Interface", interface),
            ("IP Address", address),
        ];
        for (index, (label, value)) in details.into_iter().enumerate() {
            let row = Rect::new(
                summary.origin.x,
                summary.origin.y + summary.size.height + 10.0 + index as f32 * 50.0,
                summary.size.width,
                42.0,
            );
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
                .radius(CornerRadius::Medium)
                .paint(row, context);
            Text::styled(label, TextRole::Caption)
                .weight(650)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(row.origin.x + 12.0, row.origin.y, 94.0, row.size.height),
                    context,
                );
            Text::styled(value, TextRole::Label)
                .alignment(TextAlignment::End)
                .color(Theme::current().shell.primary_text)
                .paint(
                    Rect::new(
                        row.origin.x + 106.0,
                        row.origin.y,
                        row.size.width - 118.0,
                        row.size.height,
                    ),
                    context,
                );
        }
    }

    fn paint_application_detail(
        &self,
        panel: Rect,
        bundle_id: &str,
        item_id: &str,
        fallback_title: &str,
        context: &mut PaintContext<'_>,
    ) {
        context.request_redraw_in_at(panel, Instant::now() + Duration::from_millis(100));
        let card = self
            .platform
            .borrow()
            .control_center_cards()
            .into_iter()
            .find(|card| card.bundle_id == bundle_id && card.item_id == item_id);
        let title = card
            .as_ref()
            .map(|card| card.title.as_str())
            .unwrap_or(fallback_title);
        let back = Self::back_rect(panel);
        let mut back_node = viewkit::accessibility::AccessibilityNode::new(
            viewkit::accessibility::AccessibilityRole::Button,
            back,
        );
        back_node.label = Some("Back".into());
        back_node.focusable = true;
        back_node.focused = self.interaction.borrow().focused == Some(Hit::Back);
        context.record_accessibility(back_node);
        if self.interaction.borrow().hovered == Some(Hit::Back) {
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_hover))
                .radius(CornerRadius::Medium)
                .paint(back, context);
        }
        Icon::new(SymbolName::ChevronLeft)
            .size(12.0)
            .color(Theme::current().shell.primary_text)
            .accessibility_label("Back")
            .paint(back, context);
        Text::styled(title, TextRole::TitleSmall)
            .weight(700)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    panel.origin.x + 44.0,
                    panel.origin.y + 12.0,
                    panel.size.width - 88.0,
                    28.0,
                ),
                context,
            );

        let Some(card) = card else {
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
                .radius(CornerRadius::Large)
                .paint(
                    Rect::new(
                        panel.origin.x + PANEL_PADDING,
                        panel.origin.y + 54.0,
                        panel.size.width - PANEL_PADDING * 2.0,
                        82.0,
                    ),
                    context,
                );
            Text::styled("Loading card…", TextRole::Label)
                .weight(650)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.primary_text)
                .paint(
                    Rect::new(
                        panel.origin.x + PANEL_PADDING + 12.0,
                        panel.origin.y + 68.0,
                        panel.size.width - PANEL_PADDING * 2.0 - 24.0,
                        24.0,
                    ),
                    context,
                );
            Text::styled("Waiting for the application provider", TextRole::Caption)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(
                        panel.origin.x + PANEL_PADDING + 12.0,
                        panel.origin.y + 94.0,
                        panel.size.width - PANEL_PADDING * 2.0 - 24.0,
                        20.0,
                    ),
                    context,
                );
            return;
        };

        for (index, row) in card.rows.iter().take(4).enumerate() {
            let bounds = Rect::new(
                panel.origin.x + PANEL_PADDING,
                panel.origin.y + 54.0 + index as f32 * 52.0,
                panel.size.width - PANEL_PADDING * 2.0,
                44.0,
            );
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
                .radius(CornerRadius::Medium)
                .paint(bounds, context);
            Text::styled(row.label.clone(), TextRole::Caption)
                .weight(650)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(
                        bounds.origin.x + 12.0,
                        bounds.origin.y,
                        100.0,
                        bounds.size.height,
                    ),
                    context,
                );
            Text::styled(row.value.clone(), TextRole::Label)
                .alignment(TextAlignment::End)
                .color(Theme::current().shell.primary_text)
                .paint(
                    Rect::new(
                        bounds.origin.x + 112.0,
                        bounds.origin.y,
                        bounds.size.width - 124.0,
                        bounds.size.height,
                    ),
                    context,
                );
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

    fn paint_tile(&self, item: &Item, bounds: Rect, context: &mut PaintContext<'_>) {
        let hit = Hit::Item {
            id: item.id.clone(),
        };
        let hovered = self.interaction.borrow().hovered.as_ref() == Some(&hit);
        let mut node = viewkit::accessibility::AccessibilityNode::new(
            viewkit::accessibility::AccessibilityRole::Button,
            bounds,
        );
        node.label = Some(item.title.clone());
        node.focusable = true;
        node.focused = self.interaction.borrow().focused.as_ref() == Some(&hit);
        context.record_accessibility(node);
        let focused = self.interaction.borrow().focused.as_ref() == Some(&hit);
        let pressed = self.interaction.borrow().pressed.as_ref() == Some(&hit);
        let color = if pressed {
            Theme::current().shell.control_hover
        } else if hovered || focused {
            Theme::current().colors.surface
        } else {
            Theme::current().slider.level_track
        };
        Rectangle::new()
            .color(RectangleColor::Custom(color))
            .radius(CornerRadius::Full)
            .paint(bounds, context);
        Icon::new(item.symbol)
            .size(24.0)
            .color(Theme::current().colors.text_primary)
            .accessibility_label(item.title.clone())
            .paint(bounds, context);
        if hovered {
            Text::caption(item.title.clone())
                .color(Theme::current().shell.primary_text)
                .alignment(TextAlignment::End)
                .paint(
                    Rect::new(
                        bounds.origin.x - TILE_LABEL_WIDTH - TILE_LABEL_GAP,
                        bounds.origin.y + 18.0,
                        TILE_LABEL_WIDTH,
                        24.0,
                    ),
                    context,
                );
        }
    }
    fn paint_panel(
        &self,
        panel: Rect,
        visible: &[Item],
        detail: Option<ControlCenterDetail>,
        context: &mut PaintContext<'_>,
    ) {
        if let Some(detail) = detail {
            Rectangle::new()
                .color(RectangleColor::Custom(Theme::current().card.background))
                .radius(CornerRadius::Custom(22.0))
                .paint(panel, context);
            match detail {
                ControlCenterDetail::Network => self.paint_network_detail(panel, context),
                ControlCenterDetail::Application {
                    bundle_id,
                    item_id,
                    fallback_title,
                } => self.paint_application_detail(
                    panel,
                    &bundle_id,
                    &item_id,
                    &fallback_title,
                    context,
                ),
            }
            return;
        }
        self.paint_volume(panel, context);

        let start_y = Self::tile_start_y(panel);
        for (index, item) in visible.iter().enumerate() {
            self.paint_tile(item, Self::tile_rect(panel, index, start_y), context);
        }
    }
}

impl<C: View> View for ControlCenterLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        let open = self.open.get();
        if !open && self.interaction.borrow().damage.is_none() {
            return;
        }
        let visible = self.ordered_items();
        let detail = if open {
            self.detail.get()
        } else {
            self.interaction.borrow().closing_detail.clone()
        };
        let panel = Self::panel_for_detail(bounds, visible.len(), detail.is_some());
        let margin = context
            .theme
            .shadows
            .floating
            .layers
            .iter()
            .flatten()
            .map(|shadow| {
                shadow.blur_radius
                    + shadow.spread.max(0.0)
                    + shadow.offset_x.abs().max(shadow.offset_y.abs())
            })
            .fold(TILE_LABEL_WIDTH + TILE_LABEL_GAP, f32::max)
            + 2.0;
        let grid_panel = Self::panel_for_detail(bounds, visible.len(), false);
        let detail_panel = Self::panel_for_detail(bounds, visible.len(), true);
        let scope_bounds = grid_panel.union(detail_panel);
        let current_damage = scope_bounds.expanded(margin);
        let now = Instant::now();
        let (navigation_amount, navigation_next, outgoing_detail) = {
            let mut interaction = self.interaction.borrow_mut();
            let (amount, next) =
                interaction
                    .navigation
                    .sample(&detail, context.theme.motion.standard, now);
            interaction.navigation_presented = Some((
                detail.is_some(),
                navigation_transform(amount, detail.is_some(), context.theme.spacing.extra_large),
            ));
            (amount, next, interaction.navigation.detail.clone())
        };
        let (amount, next, transform, damage) = {
            let mut interaction = self.interaction.borrow_mut();
            if open {
                interaction.closing_detail = detail.clone();
            }
            let (amount, next) = interaction.transition.sample(
                open,
                if open {
                    context.theme.motion.menu
                } else {
                    context.theme.motion.menu_dismiss
                },
                now,
            );
            let transform = PaintTransform {
                anchor: Point::new(panel.origin.x + panel.size.width, panel.origin.y),
                scale: context.theme.motion.menu_closed_scale
                    + (1.0 - context.theme.motion.menu_closed_scale) * amount,
                offset: Point::new(0.0, 0.0),
                opacity: amount,
            };
            interaction.presented = Some(transform);
            let damage = interaction
                .damage
                .map_or(current_damage, |previous| previous.union(current_damage));
            interaction.damage = (amount > 0.0 || next.is_some()).then_some(current_damage);
            if !open && next.is_none() {
                interaction.closing_detail = None;
                interaction.navigation = DetailNavigation::default();
                interaction.navigation_presented = None;
            }
            (amount, next, transform, damage)
        };
        if let Some(deadline) = next {
            context.request_redraw_in_at(damage, deadline);
        }
        if amount <= 0.0 {
            return;
        }
        if let Some(deadline) = navigation_next {
            context.request_redraw_in_at(damage, deadline);
        }
        context.with_presentation(transform, open, |context| {
            let mut scope = viewkit::accessibility::AccessibilityNode::new(
                viewkit::accessibility::AccessibilityRole::Group,
                scope_bounds,
            );
            scope.label = Some("Control Center".into());
            scope.focus_scope = true;
            context.record_accessibility(scope);
            let page = |is_detail: bool, context: &mut PaintContext<'_>| {
                let page_transform = navigation_transform(
                    navigation_amount,
                    is_detail,
                    context.theme.spacing.extra_large,
                );
                if page_transform.opacity <= 0.0 {
                    return;
                }
                context.with_presentation(
                    page_transform,
                    open && is_detail == detail.is_some(),
                    |context| {
                        if is_detail {
                            if let Some(detail) = outgoing_detail.clone() {
                                self.paint_panel(detail_panel, &visible, Some(detail), context);
                            }
                        } else {
                            self.paint_panel(grid_panel, &visible, None, context);
                        }
                    },
                );
            };
            // Paint the incoming page last; the outgoing page never participates in focus or input.
            page(detail.is_none(), context);
            page(detail.is_some(), context);
        });
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.open.get() {
            self.volume_interaction.reset();
            self.detail.set_if_changed(None);
            return self.content.handle_event(bounds, event, context);
        }
        if event
            .position()
            .is_some_and(|p| p.y < bounds.origin.y + 54.0)
        {
            return self.content.handle_event(bounds, event, context);
        }
        if matches!(event, ViewEvent::FocusChanged { focused: false }) {
            self.open.set(false);
            self.volume_interaction.reset();
            self.detail.set(None);
            context.request_redraw();
            return EventResult::Consumed;
        }
        let presented = self.interaction.borrow().presented;
        let logical_event =
            presented.map(|transform| super::popup_menu::logical_event(event, transform));
        let event = logical_event.as_ref().unwrap_or(event);
        let navigation_presented = self.interaction.borrow().navigation_presented;
        let page_event = navigation_presented
            .filter(|(detail, _)| *detail == self.detail.get().is_some())
            .map(|(_, transform)| super::popup_menu::logical_event(event, transform));
        let event = page_event.as_ref().unwrap_or(event);
        let visible = self.ordered_items();
        let panel = self.panel(bounds, visible.len());
        if let Some(detail) = self.detail.get() {
            if detail == ControlCenterDetail::Network {
                let before = self.network_enabled.get();
                let switch_result = self.network_switch.handle_event(
                    Self::network_switch_rect(panel),
                    event,
                    context,
                );
                let after = self.network_enabled.get();
                if before != after {
                    self.set_network_enabled(after);
                }
                if switch_result == EventResult::Consumed {
                    return EventResult::Consumed;
                }
            }
            match event {
                ViewEvent::KeyboardFocusRequested { bounds: target } => {
                    self.interaction.borrow_mut().focused =
                        (*target == Some(Self::back_rect(panel))).then_some(Hit::Back);
                }
                ViewEvent::KeyPressed {
                    key: Key::Enter | Key::Space,
                    ..
                } if self.interaction.borrow().focused == Some(Hit::Back) => {
                    self.detail.set(None);
                    context.request_redraw();
                }
                ViewEvent::KeyPressed {
                    key: Key::Escape, ..
                } => {
                    self.detail.set(None);
                    self.interaction.borrow_mut().hovered = None;
                    context.request_redraw();
                }
                ViewEvent::PointerMoved { position } => {
                    let hit = self.hit(bounds, *position);
                    if self.interaction.borrow().hovered != hit {
                        self.interaction.borrow_mut().hovered = hit;
                        context.request_redraw_in(panel);
                    }
                }
                ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                } => {
                    let hit = self.hit(bounds, *position);
                    if !panel.contains(*position) {
                        self.detail.set(None);
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
                    if pressed == Some(Hit::Back) && released == Some(Hit::Back) {
                        self.detail.set(None);
                    }
                    context.request_redraw_in(panel);
                }
                _ => {}
            }
            return EventResult::Consumed;
        }
        if matches!(event, ViewEvent::PointerPressed { position, button: PointerButton::Secondary } if panel.contains(*position))
        {
            self.open_customizer();
            context.request_redraw();
            return EventResult::Consumed;
        }
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
            ViewEvent::KeyboardFocusRequested { bounds: target } => {
                let focused = target.and_then(|rect| {
                    self.hit(
                        bounds,
                        Point::new(
                            rect.origin.x + rect.size.width * 0.5,
                            rect.origin.y + rect.size.height * 0.5,
                        ),
                    )
                });
                self.interaction.borrow_mut().focused = focused;
            }
            ViewEvent::KeyPressed {
                key: Key::Enter | Key::Space,
                ..
            } => {
                let focused = self.interaction.borrow().focused.clone();
                if let Some(Hit::Item { id }) = focused {
                    if let Some(item) = visible.iter().find(|item| item.id == id) {
                        self.activate(item);
                    }
                }
                context.request_redraw();
            }
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
                self.detail.set(None);
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
                if !panel.contains(*position) {
                    self.open.set(false);
                    self.detail.set(None);
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
                            let keeps_control_center_open = matches!(
                                &item.action,
                                ItemAction::ShowNetwork | ItemAction::ShowApplicationCard { .. }
                            );
                            self.activate(&item);
                            if !keeps_control_center_open {
                                self.open.set(false);
                                self.detail.set(None);
                            }
                        }
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

pub(crate) fn declares_item(app: &AppInfo, item_id: &str) -> bool {
    read_app_items(app)
        .iter()
        .any(|item| item.id == format!("app.{}:{item_id}", app.bundle_id))
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
    let mut items = Vec::new();
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
        if !valid_local_id(id) || title.is_empty() || title.len() > 40 {
            return Err("invalid item fields");
        }
        let action = match field("action") {
            Some("open-application") => ItemAction::OpenApplication(app.bundle_id.clone()),
            Some("show-card") => ItemAction::ShowApplicationCard {
                bundle_id: app.bundle_id.clone(),
                item_id: id.to_string(),
            },
            _ => return Err("invalid item fields"),
        };
        result.push(Item {
            id: format!("app.{}:{id}", app.bundle_id),
            title: title.to_string(),
            symbol,
            action,
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

    #[test]
    fn parses_application_card_registration() {
        let items = parse_app_items(
            "format=1\n[[application.control_center_items]]\nid=\"status\"\ntitle=\"Status\"\nsymbol=\"info\"\naction=\"show-card\"",
            &app(),
        )
        .unwrap();
        assert_eq!(
            items[0].action,
            ItemAction::ShowApplicationCard {
                bundle_id: String::from("org.mochios.edit"),
                item_id: String::from("status"),
            }
        );
    }
    struct TestPlatform;
    impl DesktopPlatform for TestPlatform {
        fn system_bar_state(&self) -> Result<SystemBarState, crate::platform::PlatformError> {
            Ok(SystemBarState::default())
        }
        fn open_system_settings(&self) -> Result<(), crate::platform::PlatformError> {
            Ok(())
        }
        fn perform_system_action(
            &self,
            _: SystemAction,
        ) -> Result<(), crate::platform::PlatformError> {
            Ok(())
        }
        fn refresh(&mut self) -> Result<bool, crate::platform::PlatformError> {
            Ok(false)
        }
        fn set_output_volume(
            &mut self,
            level: u8,
        ) -> Result<crate::platform::VolumeState, crate::platform::PlatformError> {
            Ok(crate::platform::VolumeState {
                level,
                ..Default::default()
            })
        }
    }

    fn layer() -> ControlCenterLayer<Rectangle> {
        ControlCenterLayer::new(
            Rectangle::new(),
            State::new(true),
            State::new(SystemBarState::default()),
            Rc::new(RefCell::new(ControlCenterPreferences::default())),
            Rc::new(RefCell::new(ControlCenterInteraction::default())),
            Rc::new(RefCell::new(TestPlatform)),
            State::new(DesktopWindows::default()),
            State::new(Vec::new()),
            Rc::new(Cell::new(None)),
        )
    }

    #[test]
    fn detail_remains_visible_during_close_without_retaining_focus() {
        use viewkit::{draw_command::DisplayList, typography::TextMeasurer, view::RedrawSchedule};
        let layer = layer();
        let mut theme = Theme::current();
        layer.detail.set(Some(ControlCenterDetail::Network));
        let now = Instant::now();
        layer.interaction.borrow_mut().navigation.sample(
            &Some(ControlCenterDetail::Network),
            theme.motion.standard,
            now - theme.motion.standard.duration * 2,
        );
        layer.interaction.borrow_mut().transition.sample(
            true,
            theme.motion.menu,
            now - theme.motion.menu.duration * 2,
        );
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut list = DisplayList::new();
        let mut nodes = Vec::new();
        let mut schedule = RedrawSchedule::new();
        let mut measurer = TextMeasurer::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes)
                .with_redraw_schedule(&mut schedule),
        );
        let before = list.commands().to_vec();
        assert!(nodes.iter().any(|node| node.focus_scope));
        layer.open.set(false);
        layer.detail.set(None); // Outside-click/Escape paths clear the live detail state immediately.
        list.clear();
        nodes.clear();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes)
                .with_redraw_schedule(&mut schedule),
        );
        assert_eq!(list.commands(), before);
        assert!(nodes.is_empty());
        assert!(schedule.deadline().is_some());
        // Reduced motion must finish even when a close is already in progress.
        theme.motion = theme.motion.reduced();
        list.clear();
        nodes.clear();
        schedule = RedrawSchedule::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes)
                .with_redraw_schedule(&mut schedule),
        );
        assert_eq!(list.commands().len(), 1);
        assert!(schedule.deadline().is_none());
        assert!(layer.interaction.borrow().closing_detail.is_none());
        assert!(layer.interaction.borrow().damage.is_none());
    }

    #[test]
    fn scaled_volume_slider_accepts_pointer_at_its_visible_edge() {
        use viewkit::typography::TextMeasurer;
        let layer = layer();
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let panel = layer.panel(bounds, 0);
        let slider = ControlCenterLayer::<Rectangle>::volume_slider_rect(panel);
        let transform = PaintTransform {
            anchor: Point::new(panel.origin.x + panel.size.width, panel.origin.y),
            scale: 0.96,
            offset: Point::new(0.0, 0.0),
            opacity: 0.5,
        };
        layer.interaction.borrow_mut().presented = Some(transform);
        let position = transform.point(Point::new(
            slider.origin.x + slider.size.width - 0.1,
            slider.origin.y + 30.0,
        ));
        assert!(!slider.contains(position));
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        layer.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert!(layer.volume_interaction.is_dragging());
        layer.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert!(!layer.volume_interaction.is_dragging());
    }
    #[test]
    fn back_transition_paints_old_detail_but_only_grid_remains_interactive() {
        use viewkit::{
            draw_command::{DisplayList, DrawCommand},
            typography::TextMeasurer,
            view::RedrawSchedule,
        };
        let layer = layer();
        let mut theme = Theme::current();
        // Keep both pages visible independent of font-layout speed on the test host.
        theme.motion.standard.duration = Duration::from_secs(60);
        let mut measurer = TextMeasurer::new();
        let now = Instant::now();
        {
            let mut interaction = layer.interaction.borrow_mut();
            interaction
                .transition
                .sample(true, theme.motion.menu, now - Duration::from_secs(2));
            interaction.navigation.sample(
                &Some(ControlCenterDetail::Network),
                theme.motion.standard,
                now - Duration::from_secs(120),
            );
            interaction.navigation.sample(
                &None,
                theme.motion.standard,
                now - Duration::from_secs(30),
            );
        }
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut list = DisplayList::new();
        let mut nodes = Vec::new();
        let mut schedule = RedrawSchedule::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes)
                .with_redraw_schedule(&mut schedule),
        );
        assert!(list.commands().iter().any(|command| matches!(command, DrawCommand::DrawText { command } if command.text == "Network")));
        assert!(
            !nodes
                .iter()
                .any(|node| node.focusable && node.label.as_deref() == Some("Back"))
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.focusable && node.label.as_deref() == Some("Volume"))
        );
        let scope = nodes.iter().find(|node| node.focus_scope).unwrap().bounds;
        assert!(schedule.deadline().is_some());
        theme.motion = theme.motion.reduced();
        list.clear();
        nodes.clear();
        schedule = RedrawSchedule::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes)
                .with_redraw_schedule(&mut schedule),
        );
        assert!(!list.commands().iter().any(|command| matches!(command, DrawCommand::DrawText { command } if command.text == "Network")));
        assert_eq!(
            nodes.iter().find(|node| node.focus_scope).unwrap().bounds,
            scope
        );
        assert!(layer.interaction.borrow().navigation.detail.is_none());
        assert!(schedule.deadline().is_none());
    }

    #[test]
    fn back_button_hit_test_composes_page_translation_and_panel_scale() {
        use viewkit::typography::TextMeasurer;
        let layer = layer();
        layer.detail.set(Some(ControlCenterDetail::Network));
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let panel = layer.panel(bounds, 0);
        let back = ControlCenterLayer::<Rectangle>::back_rect(panel);
        let theme = Theme::current();
        let outer = PaintTransform {
            anchor: Point::new(panel.origin.x + panel.size.width, panel.origin.y),
            scale: 0.96,
            offset: Point::new(0.0, 0.0),
            opacity: 0.8,
        };
        let inner = navigation_transform(0.5, true, theme.spacing.extra_large);
        {
            let mut interaction = layer.interaction.borrow_mut();
            interaction.presented = Some(outer);
            interaction.navigation_presented = Some((true, inner));
        }
        let position = outer.point(inner.point(Point::new(
            back.origin.x + back.size.width - 0.1,
            back.origin.y + back.size.height * 0.5,
        )));
        assert!(!back.contains(position));
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        layer.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        layer.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert!(layer.detail.get().is_none());
    }
}
