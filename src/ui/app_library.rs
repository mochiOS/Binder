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
const ITEM_WIDTH: f32 = 132.0;
const ITEM_HEIGHT: f32 = 108.0;
const ICON_SIZE: f32 = 60.0;
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
        let y = panel.origin.y + 76.0 + row as f32 * ITEM_HEIGHT;
        Rect::new(x, y, ITEM_WIDTH, ITEM_HEIGHT)
    }

    fn hit_index(&self, panel: Rect, position: Point) -> Option<usize> {
        self.apps
            .get()
            .iter()
            .enumerate()
            .find(|(index, _)| Self::item_rect(panel, *index).contains(position))
            .map(|(index, _)| index)
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
        let running_process = self
            .platform
            .borrow()
            .process_id_for_bundle(&app.bundle_id);
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
            return;
        }
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
        for (index, app) in self.apps.get().iter().enumerate() {
            let item = Self::item_rect(panel, index);
            if self.hovered.get() == Some(index) || self.pressed.get() == Some(index) {
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
            return self.content.handle_event(bounds, event, context);
        }
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
                } else {
                    self.pressed.set(self.hit_index(panel, *position));
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
                    self.menu_index.set(Some(index));
                    self.menu_position.set(Some(*position));
                    self.menu_action.set(None);
                    context.request_redraw_in(bounds);
                }
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
