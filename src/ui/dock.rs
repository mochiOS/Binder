use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use crate::dock_preferences::DockPreferences;
use crate::platform::{AppInfo, DesktopPlatform};

use crate::window::{DesktopWindows, ProcessActivation};
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const DOCK_HEIGHT: f32 = 70.0;
const DOCK_BOTTOM_MARGIN: f32 = 25.0;
const DOCK_HORIZONTAL_PADDING: f32 = 12.0;
const DOCK_ITEM_SIZE: f32 = 52.0;
const DOCK_ITEM_GAP: f32 = 8.0;
const DOCK_ICON_SIZE: f32 = 44.0;
const DOCK_ICON_MAX_SIZE: f32 = 60.0;
const DOCK_MAGNIFICATION_RADIUS: f32 = 96.0;
const DOCK_ICON_LIFT: f32 = 15.0;
const DOCK_PRESS_DROP: f32 = 5.0;
const DOCK_RADIUS: f32 = 48.0;
const DOCK_ITEM_RADIUS: f32 = 16.0;
const DOCK_INTERACTION_TOP_OVERFLOW: f32 = 26.0;
const DOCK_REDRAW_MARGIN: f32 = 20.0;
const WINDOW_EFFECT_EXTENT: f32 = 20.0;
const APP_LIBRARY_BUNDLE_ID: &str = "internal:app-library";

const DOCK_MENU_WIDTH: f32 = 180.0;
const DOCK_MENU_HEIGHT: f32 = 80.0;
const DOCK_MENU_GAP: f32 = 10.0;
const DOCK_MENU_REDRAW_MARGIN: f32 = 16.0;

const DOCK_TOOLTIP_HEIGHT: f32 = 30.0;
const DOCK_TOOLTIP_MARGIN: f32 = 10.0;
const DOCK_TOOLTIP_HORIZONTAL_PADDING: f32 = 12.0;
const DOCK_TOOLTIP_MAX_WIDTH: f32 = 220.0;
const DOCK_TOOLTIP_RADIUS: f32 = 12.0;
#[cfg(target_os = "mochios")]
const FALLBACK_APP_ICON: &str = "/applications/Binder.app/appicon.svg";

#[cfg(not(target_os = "mochios"))]
const FALLBACK_APP_ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/appicon.svg",);

struct DockItemVisual {
    item: Rect,
    icon: Rect,
    influence: f32,
}

#[derive(Clone)]
enum DockIcon {
    Image(ImageData),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockMenuAction {
    Open,
    Pin,
    Unpin,
}

pub(crate) struct DockLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    hovered: Rc<Cell<Option<usize>>>,
    pressed: Rc<Cell<Option<usize>>>,
    pointer: Rc<Cell<Option<Point>>>,
    running_apps: State<Vec<String>>,
    preferences: State<DockPreferences>,
    app_library_open: State<bool>,
    launch_failure_states: Rc<
        RefCell<HashMap<crate::window::WindowId, super::launch_failure::LaunchFailureWindowState>>,
    >,
    icon_cache: RefCell<HashMap<PathBuf, DockIcon>>,
    pin_menu: Menu,
    unpin_menu: Menu,
    menu_index: Rc<Cell<Option<usize>>>,
    menu_action_requested: Rc<Cell<Option<DockMenuAction>>>,
    dragged_pin: Rc<Cell<Option<usize>>>,
    drag_did_move: Rc<Cell<bool>>,
}

impl<C> DockLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
        hovered: Rc<Cell<Option<usize>>>,
        pressed: Rc<Cell<Option<usize>>>,
        pointer: Rc<Cell<Option<Point>>>,
        running_apps: State<Vec<String>>,
        preferences: State<DockPreferences>,
        app_library_open: State<bool>,
        launch_failure_states: Rc<
            RefCell<
                HashMap<crate::window::WindowId, super::launch_failure::LaunchFailureWindowState>,
            >,
        >,
    ) -> Self {
        let menu_action_requested = Rc::new(Cell::new(None));
        let open_action = Rc::clone(&menu_action_requested);
        let pin_action = Rc::clone(&menu_action_requested);
        let open_unpin_action = Rc::clone(&menu_action_requested);
        let unpin_action = Rc::clone(&menu_action_requested);

        Self {
            content,
            platform,
            windows,
            apps,
            hovered,
            pressed,
            pointer,
            running_apps,
            preferences,
            app_library_open,
            launch_failure_states,
            icon_cache: RefCell::new(HashMap::new()),
            pin_menu: Menu::new()
                .item(MenuItem::new("Open").on_select(move || {
                    open_action.set(Some(DockMenuAction::Open));
                }))
                .item(MenuItem::new("Pin to Dock").on_select(move || {
                    pin_action.set(Some(DockMenuAction::Pin));
                })),
            unpin_menu: Menu::new()
                .item(MenuItem::new("Open").on_select(move || {
                    open_unpin_action.set(Some(DockMenuAction::Open));
                }))
                .item(MenuItem::new("Unpin from Dock").on_select(move || {
                    unpin_action.set(Some(DockMenuAction::Unpin));
                })),
            menu_index: Rc::new(Cell::new(None)),
            menu_action_requested,
            dragged_pin: Rc::new(Cell::new(None)),
            drag_did_move: Rc::new(Cell::new(false)),
        }
    }

    fn active_menu(&self, index: usize) -> &Menu {
        let pinned = self
            .dock_apps()
            .get(index)
            .is_some_and(|app| self.preferences.get().is_pinned(&app.bundle_id));
        if pinned {
            &self.unpin_menu
        } else {
            &self.pin_menu
        }
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

    fn dock_apps(&self) -> Vec<AppInfo> {
        let apps = self.apps.get();
        let preferences = self.preferences.get();
        let mut visible = Vec::new();
        for bundle_id in preferences.pinned() {
            if let Some(app) = apps.iter().find(|app| &app.bundle_id == bundle_id) {
                visible.push(app.clone());
            }
        }
        for bundle_id in self.running_apps.get() {
            if preferences.is_pinned(&bundle_id) {
                continue;
            }
            if let Some(app) = apps.iter().find(|app| app.bundle_id == bundle_id) {
                visible.push(app.clone());
            }
        }
        visible.push(app_library_item());
        visible
    }

    fn pinned_visible_count(&self) -> usize {
        let apps = self.apps.get();
        self.preferences
            .get()
            .pinned()
            .iter()
            .filter(|bundle_id| apps.iter().any(|app| &app.bundle_id == *bundle_id))
            .count()
    }

    fn dock_rect(&self, bounds: Rect) -> Option<Rect> {
        let count = self.dock_apps().len();

        if count == 0 {
            return None;
        }

        let item_count = count as f32;

        let gap_count = count.saturating_sub(1) as f32;

        let width =
            DOCK_HORIZONTAL_PADDING * 2.0 + item_count * DOCK_ITEM_SIZE + gap_count * DOCK_ITEM_GAP;

        let x = bounds.origin.x + (bounds.size.width - width) / 2.0;

        let y = bounds.origin.y + bounds.size.height - DOCK_BOTTOM_MARGIN - DOCK_HEIGHT;

        Some(Rect::new(x, y, width, DOCK_HEIGHT))
    }

    fn dock_interaction_rect(dock: Rect) -> Rect {
        Rect::new(
            dock.origin.x,
            dock.origin.y - DOCK_INTERACTION_TOP_OVERFLOW,
            dock.size.width,
            dock.size.height + DOCK_INTERACTION_TOP_OVERFLOW,
        )
    }

    fn request_dock_redraw(
        &self,
        bounds: Rect,
        first: Option<usize>,
        second: Option<usize>,
        context: &mut EventContext<'_>,
    ) {
        let Some(dock) = self.dock_rect(bounds) else {
            return;
        };
        let top = DOCK_ICON_LIFT + DOCK_TOOLTIP_MARGIN + DOCK_TOOLTIP_HEIGHT + DOCK_REDRAW_MARGIN;
        let side_margin = DOCK_TOOLTIP_HORIZONTAL_PADDING + DOCK_TOOLTIP_MAX_WIDTH / 2.0;
        let vertical = Rect::new(
            dock.origin.x - side_margin,
            dock.origin.y - top,
            dock.size.width + side_margin * 2.0,
            dock.size.height + top + DOCK_REDRAW_MARGIN,
        );
        let horizontal_radius =
            DOCK_MAGNIFICATION_RADIUS + DOCK_ICON_MAX_SIZE / 2.0 + DOCK_REDRAW_MARGIN;
        let mut dirty = None;
        for index in [first, second].into_iter().flatten() {
            let item = Self::item_rect(dock, index);
            let center_x = item.origin.x + item.size.width / 2.0;
            let affected = Rect::new(
                center_x - horizontal_radius,
                vertical.origin.y,
                horizontal_radius * 2.0,
                vertical.size.height,
            )
            .intersection(vertical)
            .unwrap_or(vertical);
            dirty = Some(dirty.map_or(affected, |current: Rect| current.union(affected)));
        }
        if let Some(dirty) = dirty {
            context.request_redraw_in(dirty);
        }
    }

    fn item_rect(dock: Rect, index: usize) -> Rect {
        let x = dock.origin.x
            + DOCK_HORIZONTAL_PADDING
            + index as f32 * (DOCK_ITEM_SIZE + DOCK_ITEM_GAP);

        let y = dock.origin.y + (dock.size.height - DOCK_ITEM_SIZE) / 2.0;

        Rect::new(x, y, DOCK_ITEM_SIZE, DOCK_ITEM_SIZE)
    }

    fn hit_index(&self, bounds: Rect, position: Point) -> Option<usize> {
        let dock = self.dock_rect(bounds)?;

        if !Self::dock_interaction_rect(dock).contains(position) {
            return None;
        }

        let apps = self.dock_apps();

        for index in 0..apps.len() {
            let item = Self::item_rect(dock, index);

            if position.x >= item.origin.x && position.x <= item.origin.x + item.size.width {
                return Some(index);
            }
        }

        None
    }

    fn pointer_for_hit(&self, bounds: Rect, hit: Option<usize>) -> Option<Point> {
        let dock = self.dock_rect(bounds)?;
        let item = Self::item_rect(dock, hit?);
        Some(Point::new(
            item.origin.x + item.size.width / 2.0,
            item.origin.y + item.size.height / 2.0,
        ))
    }

    fn is_inside_dock(&self, bounds: Rect, position: Point) -> bool {
        self.dock_rect(bounds)
            .is_some_and(|dock| Self::dock_interaction_rect(dock).contains(position))
    }

    fn menu_bounds(&self, bounds: Rect) -> Option<Rect> {
        let index = self.menu_index.get()?;
        let dock = self.dock_rect(bounds)?;
        let item = Self::item_rect(dock, index);
        let center_x = item.origin.x + item.size.width / 2.0;
        let minimum_x = bounds.origin.x;
        let maximum_x = (bounds.origin.x + bounds.size.width - DOCK_MENU_WIDTH).max(minimum_x);
        let x = (center_x - DOCK_MENU_WIDTH / 2.0).clamp(minimum_x, maximum_x);
        let y = (dock.origin.y - DOCK_MENU_GAP - DOCK_MENU_HEIGHT).max(bounds.origin.y);

        Some(Rect::new(x, y, DOCK_MENU_WIDTH, DOCK_MENU_HEIGHT))
    }

    fn request_menu_redraw(&self, bounds: Rect, context: &mut EventContext<'_>) {
        if let Some(menu) = self.menu_bounds(bounds) {
            context.request_redraw_in(menu.expanded(DOCK_MENU_REDRAW_MARGIN));
        }
    }

    fn close_menu(&self, bounds: Rect, context: &mut EventContext<'_>) {
        let previous_hovered = self.hovered.get();
        self.request_menu_redraw(bounds, context);
        self.menu_index.set(None);
        self.menu_action_requested.set(None);
        self.hovered.set(None);
        self.pointer.set(None);
        self.pressed.set(None);
        self.request_dock_redraw(bounds, previous_hovered, None, context);
    }

    fn open_menu(&self, bounds: Rect, index: usize, context: &mut EventContext<'_>) {
        let previous_hovered = self.hovered.get();
        self.request_menu_redraw(bounds, context);
        self.menu_index.set(Some(index));
        self.menu_action_requested.set(None);
        self.hovered.set(Some(index));
        self.pointer.set(self.pointer_for_hit(bounds, Some(index)));
        self.pressed.set(None);
        self.request_menu_redraw(bounds, context);
        self.request_dock_redraw(bounds, previous_hovered, Some(index), context);
    }

    fn activate_with_redraw(&self, index: usize, context: &mut EventContext<'_>) {
        let before = self.visible_windows_damage();
        let activation_changed = self
            .activate_or_launch(index)
            .is_some_and(ProcessActivation::changed_window_state);
        let after = self.visible_windows_damage();
        if activation_changed || before != after {
            let dirty = match (before, self.visible_windows_damage()) {
                (Some(before), Some(after)) => Some(before.union(after)),
                (Some(dirty), None) | (None, Some(dirty)) => Some(dirty),
                (None, None) => None,
            };
            if let Some(dirty) = dirty {
                context.request_redraw_in(dirty);
            }
        }
    }

    fn magnification_influence(item: Rect, pointer: Option<Point>) -> f32 {
        let Some(pointer) = pointer else {
            return 0.0;
        };

        let center_x = item.origin.x + item.size.width / 2.0;

        let distance = (pointer.x - center_x).abs();

        if distance >= DOCK_MAGNIFICATION_RADIUS {
            return 0.0;
        }

        let t = 1.0 - distance / DOCK_MAGNIFICATION_RADIUS;

        t * t * (3.0 - 2.0 * t)
    }

    fn item_visual(
        dock: Rect,
        index: usize,
        pointer: Option<Point>,
        pressed: Option<usize>,
    ) -> DockItemVisual {
        let item = Self::item_rect(dock, index);

        let influence = Self::magnification_influence(item, pointer);

        let mut icon_size = DOCK_ICON_SIZE + (DOCK_ICON_MAX_SIZE - DOCK_ICON_SIZE) * influence;

        let mut lift = DOCK_ICON_LIFT * influence;

        if pressed == Some(index) {
            icon_size *= 0.94;
            lift = (lift - DOCK_PRESS_DROP).max(0.0);
        }

        let center_x = item.origin.x + item.size.width / 2.0;

        let center_y = item.origin.y + item.size.height / 2.0;

        let icon = Rect::new(
            center_x - icon_size / 2.0,
            center_y - icon_size / 2.0 - lift,
            icon_size,
            icon_size,
        );

        let item = Rect::new(
            item.origin.x,
            item.origin.y - lift * 0.35,
            item.size.width,
            item.size.height,
        );

        DockItemVisual {
            item,
            icon,
            influence,
        }
    }

    fn activate_or_launch(&self, index: usize) -> Option<ProcessActivation> {
        let apps = self.dock_apps();

        let Some(app) = apps.get(index).cloned() else {
            return None;
        };

        if app.bundle_id == APP_LIBRARY_BUNDLE_ID {
            self.app_library_open.update(|open| *open = !*open);
            return Some(ProcessActivation::NoWindow);
        }

        let running_process = self.platform.borrow().process_id_for_bundle(&app.bundle_id);
        let process_id = match running_process {
            Some(process_id) => process_id,
            None => match self.platform.borrow_mut().launch_app(&app) {
                Ok(process_id) => process_id,

                Err(error) => {
                    eprintln!("failed to launch app {}: {error:?}", app.bundle_id,);

                    let state = super::launch_failure::LaunchFailureWindowState::new(&app, error);
                    let mut window_id = None;
                    self.windows.update(|desktop| {
                        window_id = Some(desktop.open_launch_failure());
                    });
                    if let Some(window_id) = window_id {
                        self.launch_failure_states
                            .borrow_mut()
                            .insert(window_id, state);
                    }

                    return None;
                }
            },
        };

        let mut activation = ProcessActivation::NoWindow;
        self.windows.update(|desktop| {
            activation = desktop.activate_process(process_id);
        });
        Some(activation)
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

    fn load_icon(&self, path: &Path) -> DockIcon {
        if let Some(icon) = self.icon_cache.borrow().get(path) {
            return icon.clone();
        }

        let icon = if path.extension().and_then(|extension| extension.to_str()) == Some("svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| {
                    ImageData::from_svg(&svg, DOCK_ICON_MAX_SIZE as u32, DOCK_ICON_MAX_SIZE as u32)
                        .ok()
                })
                .map_or(DockIcon::Missing, DockIcon::Image)
        } else {
            ImageData::thumbnail_from_path(
                path,
                DOCK_ICON_MAX_SIZE as u32,
                DOCK_ICON_MAX_SIZE as u32,
            )
            .map_or(DockIcon::Missing, DockIcon::Image)
        };

        self.icon_cache
            .borrow_mut()
            .insert(path.to_path_buf(), icon.clone());
        icon
    }

    fn paint_app_icon(&self, app: &AppInfo, bounds: Rect, context: &mut PaintContext<'_>) {
        if app.bundle_id == APP_LIBRARY_BUNDLE_ID {
            self.paint_app_library_icon(bounds, context);
            return;
        }
        if let Some(icon) = &app.icon {
            if let DockIcon::Image(image) = self.load_icon(icon) {
                Image::new(image)
                    .content_mode(ImageContentMode::Fit)
                    .radius(CornerRadius::Custom(10.0))
                    .sampling(ImageSampling::Bicubic)
                    .paint(bounds, context);

                return;
            }
        }

        let fallback = PathBuf::from(FALLBACK_APP_ICON);
        if let DockIcon::Image(image) = self.load_icon(&fallback) {
            Image::new(image)
                .content_mode(ImageContentMode::Fit)
                .radius(CornerRadius::Custom(10.0))
                .sampling(ImageSampling::Bicubic)
                .paint(bounds, context);
        }
    }

    fn paint_app_library_icon(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.dock_background,
            ))
            .radius(CornerRadius::Custom(bounds.size.width * 0.3))
            .border(BorderStyle::custom(Theme::current().shell.dock_border, 1.0))
            .paint(bounds, context);

        let cell_size = bounds.size.width * 0.3;
        let gap = bounds.size.width * 0.075;
        let total = cell_size * 2.0 + gap;
        let start_x = bounds.origin.x + (bounds.size.width - total) / 2.0;
        let start_y = bounds.origin.y + (bounds.size.height - total) / 2.0;
        let apps = self.apps.get();

        for (index, app) in apps.iter().take(4).enumerate() {
            let column = index % 2;
            let row = index / 2;
            let cell = Rect::new(
                start_x + column as f32 * (cell_size + gap),
                start_y + row as f32 * (cell_size + gap),
                cell_size,
                cell_size,
            );

            Rectangle::new()
                .color(RectangleColor::Custom(
                    Theme::current().shell.dock_item_hover,
                ))
                .radius(CornerRadius::Custom(cell_size * 0.24))
                .paint(cell, context);

            let inset = cell_size * 0.08;
            let icon = Rect::new(
                cell.origin.x + inset,
                cell.origin.y + inset,
                cell.size.width - inset * 2.0,
                cell.size.height - inset * 2.0,
            );
            if let Some(path) = app.icon.as_deref()
                && let DockIcon::Image(image) = self.load_icon(path)
            {
                Image::new(image)
                    .content_mode(ImageContentMode::Fit)
                    .radius(CornerRadius::Custom(cell_size * 0.2))
                    .sampling(ImageSampling::Bicubic)
                    .paint(icon, context);
            }
        }
    }

    fn tooltip_width(text: &str) -> f32 {
        let character_count = text.chars().count() as f32;

        let text_width = character_count * 7.5;

        (text_width + DOCK_TOOLTIP_HORIZONTAL_PADDING * 2.0).clamp(48.0, DOCK_TOOLTIP_MAX_WIDTH)
    }

    fn paint_tooltip(app: &AppInfo, icon: Rect, context: &mut PaintContext<'_>) {
        let width = Self::tooltip_width(&app.name);

        let center_x = icon.origin.x + icon.size.width / 2.0;

        let tooltip = Rect::new(
            center_x - width / 2.0,
            icon.origin.y - DOCK_TOOLTIP_MARGIN - DOCK_TOOLTIP_HEIGHT,
            width,
            DOCK_TOOLTIP_HEIGHT,
        );

        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.inverse_surface,
            ))
            .radius(CornerRadius::Custom(DOCK_TOOLTIP_RADIUS))
            .paint(tooltip, context);

        Text::new(app.name.clone())
            .font_size(13.0)
            .line_height(DOCK_TOOLTIP_HEIGHT)
            .alignment(TextAlignment::Center)
            .color(Theme::current().shell.inverse_text)
            .paint(tooltip, context);
    }

    fn is_running(&self, app: &AppInfo) -> bool {
        self.running_apps
            .get()
            .iter()
            .any(|bundle_id| bundle_id == &app.bundle_id)
    }

    fn paint_running_indicator(icon: Rect, context: &mut PaintContext<'_>) {
        let size = 4.0;

        let x = icon.origin.x + icon.size.width / 2.0 - size / 2.0;

        let y = icon.origin.y + icon.size.height + 5.0;

        Ellipse::new()
            .color(EllipseColor::Custom(
                Theme::current().shell.running_indicator,
            ))
            .paint(Rect::new(x, y, size, size), context);
    }
}

impl<C> View for DockLayer<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);

        let Some(dock) = self.dock_rect(bounds) else {
            return;
        };

        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.dock_background,
            ))
            .radius(CornerRadius::Custom(DOCK_RADIUS))
            .shadow(ShadowStyle::Custom(Theme::current().shell.dock_shadow))
            .paint(dock, context);

        Rectangle::new()
            .color(RectangleColor::Custom(Color::TRANSPARENT))
            .radius(CornerRadius::Custom(DOCK_RADIUS))
            .border(BorderStyle::custom(Theme::current().shell.dock_border, 1.0))
            .paint(dock, context);

        let apps = self.dock_apps();

        let hovered = self.hovered.get();

        let pressed = self.pressed.get();

        let pointer = self.pointer.get();

        let mut tooltip: Option<(&AppInfo, Rect)> = None;

        for (index, app) in apps.iter().enumerate() {
            let visual = Self::item_visual(dock, index, pointer, pressed);

            if hovered == Some(index) || pressed == Some(index) {
                let opacity = 120.0 + 50.0 * visual.influence;
                let item = snap_rect(visual.item);

                Rectangle::new()
                    .color(RectangleColor::Custom(
                        Theme::current()
                            .shell
                            .dock_item_hover
                            .with_alpha(opacity as u8),
                    ))
                    .radius(CornerRadius::Custom(DOCK_ITEM_RADIUS))
                    .paint(item, context);
            }

            let icon = snap_rect(visual.icon);

            self.paint_app_icon(app, icon, context);

            if self.is_running(app) {
                Self::paint_running_indicator(icon, context);
            }

            if hovered == Some(index) {
                tooltip = Some((app, icon));
            }
        }

        let pinned_count = self.pinned_visible_count();
        if pinned_count > 0 && pinned_count < apps.len() {
            let left = Self::item_rect(dock, pinned_count - 1);
            let right = Self::item_rect(dock, pinned_count);
            let x = (left.origin.x + left.size.width + right.origin.x) / 2.0;
            Rectangle::new()
                .color(RectangleColor::Custom(
                    Theme::current().shell.dock_border.with_alpha(36),
                ))
                .radius(CornerRadius::Custom(1.0))
                .paint(
                    Rect::new(x, dock.origin.y + 14.0, 1.0, dock.size.height - 28.0),
                    context,
                );
        }

        if self.menu_index.get().is_none()
            && let Some((app, icon)) = tooltip
        {
            Self::paint_tooltip(app, icon, context);
        }

        if let Some(menu_bounds) = self.menu_bounds(bounds) {
            if let Some(index) = self.menu_index.get() {
                self.active_menu(index).paint(menu_bounds, context);
            }
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(menu_bounds) = self.menu_bounds(bounds) {
            match event {
                ViewEvent::KeyPressed {
                    key: Key::Escape, ..
                }
                | ViewEvent::FocusChanged { focused: false } => {
                    if let Some(index) = self.menu_index.get() {
                        self.active_menu(index).handle_event(
                            menu_bounds,
                            &ViewEvent::PointerLeft,
                            context,
                        );
                    }
                    self.close_menu(bounds, context);
                    return EventResult::Consumed;
                }
                ViewEvent::PointerMoved { position } => {
                    if menu_bounds.contains(*position) {
                        context.set_cursor(CursorIcon::Default);
                        if let Some(index) = self.menu_index.get() {
                            return self
                                .active_menu(index)
                                .handle_event(menu_bounds, event, context)
                                .merge(EventResult::Consumed);
                        }
                        return EventResult::Consumed;
                    }

                    if let Some(index) = self.menu_index.get() {
                        self.active_menu(index).handle_event(
                            menu_bounds,
                            &ViewEvent::PointerLeft,
                            context,
                        );
                    }
                    return EventResult::Consumed;
                }
                ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                } => {
                    if menu_bounds.contains(*position) {
                        if let Some(index) = self.menu_index.get() {
                            return self
                                .active_menu(index)
                                .handle_event(menu_bounds, event, context)
                                .merge(EventResult::Consumed);
                        }
                        return EventResult::Consumed;
                    }

                    self.close_menu(bounds, context);
                    return EventResult::Consumed;
                }
                ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                } => {
                    if !menu_bounds.contains(*position) {
                        return EventResult::Consumed;
                    }

                    let index = self.menu_index.get();
                    let result = index.map_or(EventResult::Ignored, |index| {
                        self.active_menu(index)
                            .handle_event(menu_bounds, event, context)
                    });
                    if let Some(action) = self.menu_action_requested.replace(None) {
                        let app = index.and_then(|index| self.dock_apps().get(index).cloned());
                        self.close_menu(bounds, context);
                        if let Some(app) = app {
                            match action {
                                DockMenuAction::Open => {
                                    if let Some(index) = self
                                        .dock_apps()
                                        .iter()
                                        .position(|candidate| candidate.bundle_id == app.bundle_id)
                                    {
                                        self.activate_with_redraw(index, context);
                                    }
                                }
                                DockMenuAction::Pin => self.update_pin(&app.bundle_id, true),
                                DockMenuAction::Unpin => self.update_pin(&app.bundle_id, false),
                            }
                            context.request_redraw_in(bounds);
                        }
                    }
                    return result.merge(EventResult::Consumed);
                }
                ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Secondary,
                } => {
                    if let Some(index) = self.menu_index.get() {
                        self.active_menu(index).handle_event(
                            menu_bounds,
                            &ViewEvent::PointerLeft,
                            context,
                        );
                    }
                    if let Some(index) = self.hit_index(bounds, *position) {
                        self.open_menu(bounds, index, context);
                    } else {
                        self.close_menu(bounds, context);
                    }
                    return EventResult::Consumed;
                }
                ViewEvent::PointerLeft => {
                    if let Some(index) = self.menu_index.get() {
                        return self
                            .active_menu(index)
                            .handle_event(menu_bounds, event, context)
                            .merge(EventResult::Consumed);
                    }
                    return EventResult::Consumed;
                }
                _ => return EventResult::Consumed,
            }
        }

        match event {
            ViewEvent::PointerMoved { position } => {
                let inside = self.is_inside_dock(bounds, *position);

                let hit = if inside {
                    self.hit_index(bounds, *position)
                } else {
                    None
                };

                let previous_hovered = self.hovered.get();
                let changed = previous_hovered != hit;

                if changed {
                    self.hovered.set(hit);
                    self.pointer.set(self.pointer_for_hit(bounds, hit));
                    self.request_dock_redraw(bounds, previous_hovered, hit, context);
                }

                if let (Some(from), Some(to)) = (self.dragged_pin.get(), hit) {
                    let pinned_count = self.pinned_visible_count();
                    if from < pinned_count && to < pinned_count && from != to {
                        let dock_apps = self.dock_apps();
                        let from_bundle = dock_apps.get(from).map(|app| app.bundle_id.clone());
                        let to_bundle = dock_apps.get(to).map(|app| app.bundle_id.clone());
                        let mut moved = false;
                        if let (Some(from_bundle), Some(to_bundle)) = (from_bundle, to_bundle) {
                            self.preferences.update(|preferences| {
                                moved = preferences.move_pin_bundle(&from_bundle, &to_bundle);
                                if moved && let Err(error) = preferences.save() {
                                    eprintln!("failed to save Dock preferences: {error}");
                                }
                            });
                        }
                        if moved {
                            self.drag_did_move.set(true);
                            self.dragged_pin.set(Some(to));
                            self.pressed.set(Some(to));
                            context.request_redraw_in(bounds);
                        }
                    }
                }

                if inside {
                    context.set_cursor(CursorIcon::Pointer);

                    return EventResult::Consumed;
                }

                self.content.handle_event(bounds, event, context)
            }

            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if self.is_inside_dock(bounds, *position) {
                    let hit = self.hit_index(bounds, *position);
                    self.pressed.set(hit);
                    self.dragged_pin
                        .set(hit.filter(|index| *index < self.pinned_visible_count()));
                    self.drag_did_move.set(false);

                    self.request_dock_redraw(bounds, hit, None, context);

                    return EventResult::Consumed;
                }

                self.content.handle_event(bounds, event, context)
            }

            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Secondary,
            } => {
                if let Some(index) = self.hit_index(bounds, *position) {
                    if self
                        .dock_apps()
                        .get(index)
                        .is_some_and(|app| app.bundle_id != APP_LIBRARY_BUNDLE_ID)
                    {
                        self.open_menu(bounds, index, context);
                        return EventResult::Consumed;
                    }
                }

                self.content.handle_event(bounds, event, context)
            }

            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let pressed = self.pressed.get();

                let hit = self.hit_index(bounds, *position);

                if pressed.is_some() {
                    self.pressed.set(None);

                    self.dragged_pin.set(None);
                    let drag_did_move = self.drag_did_move.replace(false);

                    if !drag_did_move && pressed == hit {
                        if let Some(index) = hit {
                            self.activate_with_redraw(index, context);
                        }
                    }

                    self.request_dock_redraw(bounds, pressed, hit, context);

                    return EventResult::Consumed;
                }

                if self.is_inside_dock(bounds, *position) {
                    return EventResult::Consumed;
                }

                self.content.handle_event(bounds, event, context)
            }

            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                let previous_hovered = self.hovered.get();
                let previous_pressed = self.pressed.get();
                let changed = self.hovered.get().is_some()
                    || self.pressed.get().is_some()
                    || self.pointer.get().is_some();

                if self.hovered.get().is_some() {
                    self.hovered.set(None);
                }
                if self.pressed.get().is_some() {
                    self.pressed.set(None);
                }
                self.dragged_pin.set(None);
                self.drag_did_move.set(false);
                if self.pointer.get().is_some() {
                    self.pointer.set(None);
                }

                if changed {
                    self.request_dock_redraw(bounds, previous_hovered, previous_pressed, context);
                }

                self.content.handle_event(bounds, event, context)
            }

            _ => {
                if let Some(position) = event.position() {
                    if self.is_inside_dock(bounds, position) {
                        return EventResult::Consumed;
                    }
                }

                self.content.handle_event(bounds, event, context)
            }
        }
    }
}

fn snap_rect(rect: Rect) -> Rect {
    let x = rect.origin.x.round();
    let y = rect.origin.y.round();

    let width = rect.size.width.round().max(1.0);
    let height = rect.size.height.round().max(1.0);

    Rect::new(x, y, width, height)
}

fn app_library_item() -> AppInfo {
    AppInfo {
        root: PathBuf::new(),
        name: String::from("App Library"),
        bundle_id: String::from(APP_LIBRARY_BUNDLE_ID),
        version: String::new(),
        developer: String::new(),
        entry: String::new(),
        description: String::new(),
        icon: None,
        resources: Vec::new(),
    }
}
