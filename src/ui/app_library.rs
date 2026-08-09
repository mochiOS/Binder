use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::dock_preferences::DockPreferences;
use crate::platform::{AppInfo, DesktopPlatform};
use crate::window::{DesktopWindows, ProcessActivation};
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    theme::{Shadow, ShadowSet},
    view::{Constraints, MeasureContext, PaintContext},
};

const PANEL_WIDTH: f32 = 760.0;
const PANEL_HEIGHT: f32 = 520.0;
const GRID_COLUMNS: usize = 5;
const GRID_ROWS: usize = 3;
const PAGE_SIZE: usize = GRID_COLUMNS * GRID_ROWS;
const ITEM_WIDTH: f32 = 132.0;
const ITEM_HEIGHT: f32 = 108.0;
const ICON_SIZE: f32 = 60.0;
const SEARCH_WIDTH: f32 = 310.0;
const SEARCH_HEIGHT: f32 = 38.0;
const PAGE_BUTTON_SIZE: f32 = 30.0;
const MENU_WIDTH: f32 = 190.0;
const MENU_HEIGHT: f32 = 80.0;
const WINDOW_EFFECT_EXTENT: f32 = 20.0;

#[cfg(target_os = "mochios")]
const FALLBACK_APP_ICON: &str = "/applications/Binder.app/appicon.svg";

#[cfg(not(target_os = "mochios"))]
const FALLBACK_APP_ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/appicon.svg");

const PANEL_SHADOW: ShadowSet =
    ShadowSet::single(Shadow::new(Color::rgba(0, 0, 0, 44), 0.0, 12.0, 32.0, 0.0));

#[derive(Clone)]
enum CachedIcon {
    Image(ImageData),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    Open,
    Pin,
    Unpin,
}

pub(crate) struct AppLibraryLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    preferences: State<DockPreferences>,
    open: State<bool>,
    launch_failure_states: Rc<
        RefCell<HashMap<crate::window::WindowId, super::launch_failure::LaunchFailureWindowState>>,
    >,
    hovered: Cell<Option<usize>>,
    pressed: Cell<Option<usize>>,
    selected: Cell<Option<usize>>,
    query: RefCell<String>,
    page: Cell<usize>,
    was_open: Cell<bool>,
    menu_index: Cell<Option<usize>>,
    menu_position: Cell<Option<Point>>,
    menu_action: Rc<Cell<Option<MenuAction>>>,
    pin_menu: Menu,
    unpin_menu: Menu,
    icon_cache: RefCell<HashMap<PathBuf, CachedIcon>>,
}

impl<C> AppLibraryLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
        preferences: State<DockPreferences>,
        open: State<bool>,
        launch_failure_states: Rc<
            RefCell<
                HashMap<crate::window::WindowId, super::launch_failure::LaunchFailureWindowState>,
            >,
        >,
    ) -> Self {
        let menu_action = Rc::new(Cell::new(None));
        let open_action = Rc::clone(&menu_action);
        let pin_action = Rc::clone(&menu_action);
        let open_unpin_action = Rc::clone(&menu_action);
        let unpin_action = Rc::clone(&menu_action);
        Self {
            content,
            platform,
            windows,
            apps,
            preferences,
            open,
            launch_failure_states,
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            selected: Cell::new(None),
            query: RefCell::new(String::new()),
            page: Cell::new(0),
            was_open: Cell::new(false),
            menu_index: Cell::new(None),
            menu_position: Cell::new(None),
            menu_action,
            pin_menu: Menu::new()
                .item(MenuItem::new("Open").on_select(move || {
                    open_action.set(Some(MenuAction::Open));
                }))
                .item(MenuItem::new("Pin to Dock").on_select(move || {
                    pin_action.set(Some(MenuAction::Pin));
                })),
            unpin_menu: Menu::new()
                .item(MenuItem::new("Open").on_select(move || {
                    open_unpin_action.set(Some(MenuAction::Open));
                }))
                .item(MenuItem::new("Unpin from Dock").on_select(move || {
                    unpin_action.set(Some(MenuAction::Unpin));
                })),
            icon_cache: RefCell::new(HashMap::new()),
        }
    }

    fn panel_rect(bounds: Rect) -> Rect {
        let width = PANEL_WIDTH.min(bounds.size.width - 64.0).max(320.0);
        let height = PANEL_HEIGHT.min(bounds.size.height - 120.0).max(280.0);
        Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - height) / 2.0 - 24.0,
            width,
            height,
        )
    }

    fn item_rect(panel: Rect, index: usize) -> Rect {
        let columns = GRID_COLUMNS;
        let row = index / columns;
        let column = index % columns;
        let grid_width = ITEM_WIDTH * columns as f32;
        let x = panel.origin.x + (panel.size.width - grid_width) / 2.0 + column as f32 * ITEM_WIDTH;
        let y = panel.origin.y + 84.0 + row as f32 * ITEM_HEIGHT;
        Rect::new(x, y, ITEM_WIDTH, ITEM_HEIGHT)
    }

    fn search_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width - SEARCH_WIDTH - 30.0,
            panel.origin.y + 22.0,
            SEARCH_WIDTH,
            SEARCH_HEIGHT,
        )
    }

    fn previous_page_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width / 2.0 - 78.0,
            panel.origin.y + panel.size.height - 46.0,
            PAGE_BUTTON_SIZE,
            PAGE_BUTTON_SIZE,
        )
    }

    fn next_page_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width / 2.0 + 48.0,
            panel.origin.y + panel.size.height - 46.0,
            PAGE_BUTTON_SIZE,
            PAGE_BUTTON_SIZE,
        )
    }

    fn filtered_indices(&self) -> Vec<usize> {
        matching_app_indices(&self.apps.get(), &self.query.borrow())
    }

    fn page_count(&self, filtered: &[usize]) -> usize {
        page_count(filtered.len())
    }

    fn clamp_page(&self, filtered: &[usize]) {
        let last_page = self.page_count(filtered).saturating_sub(1);
        if self.page.get() > last_page {
            self.page.set(last_page);
        }
    }

    fn visible_indices(&self, filtered: &[usize]) -> Vec<usize> {
        let start = self.page.get().saturating_mul(PAGE_SIZE);
        filtered
            .iter()
            .skip(start)
            .take(PAGE_SIZE)
            .copied()
            .collect()
    }

    fn reset_selection(&self) {
        let filtered = self.filtered_indices();
        self.page.set(0);
        self.selected.set(filtered.first().copied());
        self.hovered.set(None);
        self.pressed.set(None);
        self.close_menu();
    }

    fn ensure_open_session(&self) {
        if self.was_open.replace(true) {
            return;
        }
        self.query.borrow_mut().clear();
        self.reset_selection();
        self.selected.set(None);
    }

    fn select_page(&self, page: usize) {
        let filtered = self.filtered_indices();
        let last_page = self.page_count(&filtered).saturating_sub(1);
        let page = page.min(last_page);
        self.page.set(page);
        self.selected
            .set(filtered.get(page.saturating_mul(PAGE_SIZE)).copied());
        self.hovered.set(None);
        self.pressed.set(None);
        self.close_menu();
    }

    fn move_selection(&self, offset: isize) {
        let filtered = self.filtered_indices();
        let selected = adjacent_selection(&filtered, self.selected.get(), offset);
        self.selected.set(selected);
        if let Some(selected) = selected
            && let Some(position) = filtered.iter().position(|index| *index == selected)
        {
            self.page.set(position / PAGE_SIZE);
        }
        self.hovered.set(None);
    }

    fn hit_index(&self, panel: Rect, position: Point) -> Option<usize> {
        let filtered = self.filtered_indices();
        self.visible_indices(&filtered)
            .into_iter()
            .enumerate()
            .find(|(local_index, _)| Self::item_rect(panel, *local_index).contains(position))
            .map(|(_, app_index)| app_index)
    }

    fn menu_bounds(&self, bounds: Rect) -> Option<Rect> {
        let position = self.menu_position.get()?;
        let x = position.x.clamp(
            bounds.origin.x,
            bounds.origin.x + bounds.size.width - MENU_WIDTH,
        );
        let y = position.y.clamp(
            bounds.origin.y,
            bounds.origin.y + bounds.size.height - MENU_HEIGHT,
        );
        Some(Rect::new(x, y, MENU_WIDTH, MENU_HEIGHT))
    }

    fn active_menu(&self) -> &Menu {
        let pinned = self
            .menu_index
            .get()
            .and_then(|index| self.apps.get().get(index).cloned())
            .is_some_and(|app| self.preferences.get().is_pinned(&app.bundle_id));
        if pinned {
            &self.unpin_menu
        } else {
            &self.pin_menu
        }
    }

    fn close_menu(&self) {
        self.menu_index.set(None);
        self.menu_position.set(None);
        self.menu_action.set(None);
    }

    fn apply_menu_action(&self, context: &mut EventContext<'_>, bounds: Rect) {
        let Some(action) = self.menu_action.replace(None) else {
            return;
        };
        let app = self
            .menu_index
            .get()
            .and_then(|index| self.apps.get().get(index).cloned());
        self.close_menu();
        let Some(app) = app else {
            return;
        };
        match action {
            MenuAction::Open => self.activate(&app, context),
            MenuAction::Pin => self.update_pin(&app.bundle_id, true),
            MenuAction::Unpin => self.update_pin(&app.bundle_id, false),
        }
        context.request_redraw_in(bounds);
    }

    fn update_pin(&self, bundle_id: &str, pin: bool) {
        self.preferences.update(|preferences| {
            let changed = if pin {
                preferences.pin(bundle_id)
            } else {
                preferences.unpin(bundle_id)
            };
            if changed && let Err(error) = preferences.save() {
                eprintln!("failed to save Dock preferences: {error}");
            }
        });
    }

    fn activate(&self, app: &AppInfo, context: &mut EventContext<'_>) {
        let before = self.visible_windows_damage();
        let running_process = self.platform.borrow().process_id_for_bundle(&app.bundle_id);
        let process_id = match running_process {
            Some(process_id) => process_id,
            None => match self.platform.borrow_mut().launch_app(app) {
                Ok(process_id) => process_id,
                Err(error) => {
                    eprintln!("failed to launch app {}: {error:?}", app.bundle_id);
                    let state = super::launch_failure::LaunchFailureWindowState::new(app, error);
                    let mut window_id = None;
                    self.windows.update(|desktop| {
                        window_id = Some(desktop.open_launch_failure());
                    });
                    if let Some(window_id) = window_id {
                        self.launch_failure_states
                            .borrow_mut()
                            .insert(window_id, state);
                    }
                    self.open.set(false);
                    context.request_redraw_in(bounds_for_damage(
                        before,
                        self.visible_windows_damage(),
                    ));
                    return;
                }
            },
        };
        let mut activation = ProcessActivation::NoWindow;
        self.windows
            .update(|desktop| activation = desktop.activate_process(process_id));
        let after = self.visible_windows_damage();
        if activation.changed_window_state() || before != after {
            context.request_redraw_in(bounds_for_damage(before, after));
        }
        self.open.set(false);
    }

    fn visible_windows_damage(&self) -> Option<Rect> {
        self.windows
            .get()
            .windows
            .iter()
            .filter(|window| !window.minimized)
            .map(|window| window.frame.expanded(WINDOW_EFFECT_EXTENT))
            .reduce(Rect::union)
    }

    fn load_icon(&self, path: &Path) -> CachedIcon {
        if let Some(icon) = self.icon_cache.borrow().get(path) {
            return icon.clone();
        }
        let icon = if path.extension().and_then(|extension| extension.to_str()) == Some("svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| ImageData::from_svg(&svg, 72, 72).ok())
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        } else {
            ImageData::thumbnail_from_path(path, 72, 72)
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        };
        self.icon_cache
            .borrow_mut()
            .insert(path.to_path_buf(), icon.clone());
        icon
    }

    fn paint_icon(&self, app: &AppInfo, bounds: Rect, context: &mut PaintContext<'_>) {
        let path = app
            .icon
            .as_deref()
            .unwrap_or_else(|| Path::new(FALLBACK_APP_ICON));
        if let CachedIcon::Image(image) = self.load_icon(path) {
            Image::new(image)
                .content_mode(ImageContentMode::Fit)
                .sampling(ImageSampling::Bicubic)
                .radius(CornerRadius::Custom(13.0))
                .paint(bounds, context);
        }
    }
}

impl<C> View for AppLibraryLayer<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        if !self.open.get() {
            self.was_open.set(false);
            return;
        }
        self.ensure_open_session();
        Rectangle::new()
            .color(RectangleColor::Custom(Color::rgba(0, 0, 0, 72)))
            .paint(bounds, context);
        let panel = Self::panel_rect(bounds);
        Rectangle::new()
            .color(RectangleColor::Custom(Color::rgba(247, 247, 249, 246)))
            .radius(CornerRadius::Custom(22.0))
            .shadow(ShadowStyle::Custom(PANEL_SHADOW))
            .border(BorderStyle::custom(Color::rgba(255, 255, 255, 180), 1.0))
            .paint(panel, context);
        Text::new("Applications")
            .font_size(24.0)
            .line_height(34.0)
            .weight(700)
            .paint(
                Rect::new(panel.origin.x + 34.0, panel.origin.y + 24.0, 300.0, 36.0),
                context,
            );
        let search = Self::search_rect(panel);
        Rectangle::new()
            .color(RectangleColor::Custom(Color::rgba(255, 255, 255, 232)))
            .radius(CornerRadius::Custom(11.0))
            .border(BorderStyle::custom(Color::rgba(0, 0, 0, 30), 1.0))
            .paint(search, context);
        let query = self.query.borrow().clone();
        Text::new(if query.is_empty() {
            String::from("Search applications")
        } else {
            query
        })
        .font_size(13.0)
        .line_height(20.0)
        .color(if self.query.borrow().is_empty() {
            Color::rgba(70, 70, 74, 150)
        } else {
            Color::from_rgb_hex(0x1d1d1f)
        })
        .paint(
            Rect::new(
                search.origin.x + 14.0,
                search.origin.y + 9.0,
                search.size.width - 28.0,
                22.0,
            ),
            context,
        );

        let apps = self.apps.get();
        let filtered = matching_app_indices(&apps, &self.query.borrow());
        self.clamp_page(&filtered);
        let visible = self.visible_indices(&filtered);
        for (local_index, app_index) in visible.iter().copied().enumerate() {
            let Some(app) = apps.get(app_index) else {
                continue;
            };
            let item = Self::item_rect(panel, local_index);
            if self.selected.get() == Some(app_index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(Color::rgba(0, 122, 255, 28)))
                    .radius(CornerRadius::Custom(14.0))
                    .border(BorderStyle::custom(Color::rgba(0, 122, 255, 72), 1.0))
                    .paint(item, context);
            } else if self.hovered.get() == Some(app_index) || self.pressed.get() == Some(app_index)
            {
                Rectangle::new()
                    .color(RectangleColor::Custom(Color::rgba(0, 0, 0, 12)))
                    .radius(CornerRadius::Custom(14.0))
                    .paint(item, context);
            }
            let icon = Rect::new(
                item.origin.x + (item.size.width - ICON_SIZE) / 2.0,
                item.origin.y + 6.0,
                ICON_SIZE,
                ICON_SIZE,
            );
            self.paint_icon(app, icon, context);
            Text::new(app.name.clone())
                .font_size(12.0)
                .line_height(20.0)
                .alignment(TextAlignment::Center)
                .paint(
                    Rect::new(
                        item.origin.x + 4.0,
                        item.origin.y + 70.0,
                        item.size.width - 8.0,
                        30.0,
                    ),
                    context,
                );
        }

        if filtered.is_empty() {
            Text::new("No applications found")
                .font_size(14.0)
                .line_height(22.0)
                .alignment(TextAlignment::Center)
                .color(Color::rgba(60, 60, 67, 170))
                .paint(
                    Rect::new(
                        panel.origin.x + 80.0,
                        panel.origin.y + 220.0,
                        panel.size.width - 160.0,
                        28.0,
                    ),
                    context,
                );
        }

        let page_count = self.page_count(&filtered);
        let page = self.page.get();
        let previous = Self::previous_page_rect(panel);
        let next = Self::next_page_rect(panel);
        for (button, enabled, label) in [
            (previous, page > 0, "<"),
            (next, page + 1 < page_count, ">"),
        ] {
            Rectangle::new()
                .color(RectangleColor::Custom(if enabled {
                    Color::rgba(0, 0, 0, 14)
                } else {
                    Color::rgba(0, 0, 0, 5)
                }))
                .radius(CornerRadius::Custom(PAGE_BUTTON_SIZE / 2.0))
                .paint(button, context);
            Text::new(label)
                .font_size(15.0)
                .line_height(22.0)
                .weight(700)
                .alignment(TextAlignment::Center)
                .color(if enabled {
                    Color::from_rgb_hex(0x1d1d1f)
                } else {
                    Color::rgba(60, 60, 67, 70)
                })
                .paint(
                    Rect::new(
                        button.origin.x,
                        button.origin.y + 4.0,
                        button.size.width,
                        22.0,
                    ),
                    context,
                );
        }
        Text::new(format!("{} / {}", page + 1, page_count))
            .font_size(11.0)
            .line_height(18.0)
            .alignment(TextAlignment::Center)
            .color(Color::rgba(60, 60, 67, 170))
            .paint(
                Rect::new(
                    panel.origin.x + panel.size.width / 2.0 - 42.0,
                    panel.origin.y + panel.size.height - 40.0,
                    84.0,
                    20.0,
                ),
                context,
            );
        if let Some(menu_bounds) = self.menu_bounds(bounds) {
            self.active_menu().paint(menu_bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.open.get() {
            self.was_open.set(false);
            return self.content.handle_event(bounds, event, context);
        }
        self.ensure_open_session();
        if let Some(menu_bounds) = self.menu_bounds(bounds) {
            match event {
                ViewEvent::PointerMoved { position }
                | ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                }
                | ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                } if menu_bounds.contains(*position) => {
                    let result = self.active_menu().handle_event(menu_bounds, event, context);
                    self.apply_menu_action(context, bounds);
                    return result.merge(EventResult::Consumed);
                }
                ViewEvent::KeyPressed {
                    key: Key::Escape, ..
                } => {
                    self.close_menu();
                    context.request_redraw_in(bounds);
                    return EventResult::Consumed;
                }
                ViewEvent::PointerPressed { .. } => {
                    self.close_menu();
                    context.request_redraw_in(bounds);
                    return EventResult::Consumed;
                }
                _ => return EventResult::Consumed,
            }
        }
        let panel = Self::panel_rect(bounds);
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
                context.request_redraw_in(bounds);
            }
            ViewEvent::ArrowLeft => {
                self.move_selection(-1);
                context.request_redraw_in(panel);
            }
            ViewEvent::ArrowRight => {
                self.move_selection(1);
                context.request_redraw_in(panel);
            }
            ViewEvent::KeyPressed {
                key: Key::ArrowUp, ..
            } => {
                self.move_selection(-(GRID_COLUMNS as isize));
                context.request_redraw_in(panel);
            }
            ViewEvent::KeyPressed {
                key: Key::ArrowDown,
                ..
            } => {
                self.move_selection(GRID_COLUMNS as isize);
                context.request_redraw_in(panel);
            }
            ViewEvent::KeyPressed {
                key: Key::PageUp, ..
            } => {
                self.select_page(self.page.get().saturating_sub(1));
                context.request_redraw_in(panel);
            }
            ViewEvent::KeyPressed {
                key: Key::PageDown, ..
            } => {
                self.select_page(self.page.get().saturating_add(1));
                context.request_redraw_in(panel);
            }
            ViewEvent::Backspace => {
                self.query.borrow_mut().pop();
                self.reset_selection();
                context.request_redraw_in(panel);
            }
            ViewEvent::TextInput { text } if text.contains(['\r', '\n']) => {
                let app = self
                    .selected
                    .get()
                    .and_then(|index| self.apps.get().get(index).cloned());
                if let Some(app) = app {
                    self.activate(&app, context);
                }
            }
            ViewEvent::TextInput { text } => {
                let mut query = self.query.borrow_mut();
                for character in text.chars().filter(|character| !character.is_control()) {
                    if query.chars().count() >= 64 {
                        break;
                    }
                    query.push(character);
                }
                drop(query);
                self.reset_selection();
                context.request_redraw_in(panel);
            }
            ViewEvent::PointerMoved { position } => {
                self.hovered.set(self.hit_index(panel, *position));
                context.set_cursor(if self.hovered.get().is_some() {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                });
                context.request_redraw_in(panel);
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if !panel.contains(*position) {
                    self.open.set(false);
                    context.request_redraw_in(bounds);
                } else if Self::previous_page_rect(panel).contains(*position) {
                    self.select_page(self.page.get().saturating_sub(1));
                    context.request_redraw_in(panel);
                } else if Self::next_page_rect(panel).contains(*position) {
                    self.select_page(self.page.get().saturating_add(1));
                    context.request_redraw_in(panel);
                } else {
                    self.pressed.set(self.hit_index(panel, *position));
                    self.selected.set(self.pressed.get());
                    context.request_redraw_in(panel);
                }
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let pressed = self.pressed.replace(None);
                let hit = self.hit_index(panel, *position);
                if pressed == hit
                    && let Some(app) = hit.and_then(|index| self.apps.get().get(index).cloned())
                {
                    self.activate(&app, context);
                }
                context.request_redraw_in(bounds);
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Secondary,
            } => {
                if let Some(index) = self.hit_index(panel, *position) {
                    self.selected.set(Some(index));
                    self.menu_index.set(Some(index));
                    self.menu_position.set(Some(*position));
                    self.menu_action.set(None);
                    context.request_redraw_in(bounds);
                }
            }
            ViewEvent::Scroll {
                position, delta_y, ..
            } if panel.contains(*position) && *delta_y != 0.0 => {
                if *delta_y > 0.0 {
                    self.select_page(self.page.get().saturating_sub(1));
                } else {
                    self.select_page(self.page.get().saturating_add(1));
                }
                context.request_redraw_in(panel);
            }
            _ => {}
        }
        EventResult::Consumed
    }
}

fn bounds_for_damage(before: Option<Rect>, after: Option<Rect>) -> Rect {
    match (before, after) {
        (Some(before), Some(after)) => before.union(after),
        (Some(bounds), None) | (None, Some(bounds)) => bounds,
        (None, None) => Rect::new(0.0, 0.0, 1.0, 1.0),
    }
}

fn matching_app_indices(apps: &[AppInfo], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    apps.iter()
        .enumerate()
        .filter_map(|(index, app)| {
            let matches = query.is_empty()
                || app.name.to_lowercase().contains(&query)
                || app.bundle_id.to_lowercase().contains(&query)
                || app.developer.to_lowercase().contains(&query);
            matches.then_some(index)
        })
        .collect()
}

fn page_count(item_count: usize) -> usize {
    item_count.div_ceil(PAGE_SIZE).max(1)
}

fn adjacent_selection(filtered: &[usize], selected: Option<usize>, offset: isize) -> Option<usize> {
    let Some(current) =
        selected.and_then(|selected| filtered.iter().position(|index| *index == selected))
    else {
        return filtered.first().copied();
    };
    let target = current
        .saturating_add_signed(offset)
        .min(filtered.len().saturating_sub(1));
    filtered.get(target).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle_id: &str, developer: &str) -> AppInfo {
        AppInfo {
            root: PathBuf::new(),
            name: name.to_owned(),
            bundle_id: bundle_id.to_owned(),
            version: String::from("1.0.0"),
            developer: developer.to_owned(),
            entry: String::from("entry.elf"),
            description: String::new(),
            icon: None,
            resources: Vec::new(),
        }
    }

    #[test]
    fn search_matches_name_bundle_and_developer_case_insensitively() {
        let apps = vec![
            app("Files", "org.mochios.files", "mochiOS"),
            app("Terminal", "org.mochios.terminal", "System Team"),
        ];
        assert_eq!(matching_app_indices(&apps, "FILE"), vec![0]);
        assert_eq!(matching_app_indices(&apps, "terminal"), vec![1]);
        assert_eq!(matching_app_indices(&apps, "system team"), vec![1]);
        assert_eq!(matching_app_indices(&apps, "missing"), Vec::<usize>::new());
    }

    #[test]
    fn page_count_keeps_an_empty_page_and_rounds_up() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(PAGE_SIZE), 1);
        assert_eq!(page_count(PAGE_SIZE + 1), 2);
    }

    #[test]
    fn selection_navigation_uses_filtered_app_indices() {
        let filtered = vec![2, 5, 9];
        assert_eq!(adjacent_selection(&filtered, None, 1), Some(2));
        assert_eq!(adjacent_selection(&filtered, Some(5), 1), Some(9));
        assert_eq!(adjacent_selection(&filtered, Some(5), -1), Some(2));
        assert_eq!(adjacent_selection(&filtered, Some(9), 1), Some(9));
        assert_eq!(adjacent_selection(&[], None, 1), None);
    }
}
