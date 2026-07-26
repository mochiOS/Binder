use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform};

use crate::window::DesktopWindows;
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    theme::{Shadow, ShadowSet},
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

const DOCK_TOOLTIP_HEIGHT: f32 = 30.0;
const DOCK_TOOLTIP_MARGIN: f32 = 10.0;
const DOCK_TOOLTIP_HORIZONTAL_PADDING: f32 = 12.0;
const DOCK_TOOLTIP_MAX_WIDTH: f32 = 220.0;
const DOCK_TOOLTIP_RADIUS: f32 = 12.0;
const DOCK_TOOLTIP_BACKGROUND: Color = Color::rgba(38, 38, 38, 230);
const DOCK_TOOLTIP_TEXT: Color = Color::rgba(255, 255, 255, 255);

const DOCK_BACKGROUND: Color = Color::rgba(255, 255, 255, 190);

const DOCK_BORDER: Color = Color::rgba(0, 0, 0, 28);

#[cfg(target_os = "mochios")]
const FALLBACK_APP_ICON: &str = "/applications/Binder.app/appicon.svg";

#[cfg(not(target_os = "mochios"))]
const FALLBACK_APP_ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/appicon.svg",);

const DOCK_SHADOW: ShadowSet =
    ShadowSet::single(Shadow::new(Color::rgba(0, 0, 0, 18), 0.0, 4.0, 12.0, 0.0));

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

pub(crate) struct DockLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    hovered: Rc<Cell<Option<usize>>>,
    pressed: Rc<Cell<Option<usize>>>,
    pointer: Rc<Cell<Option<Point>>>,
    running_apps: State<Vec<String>>,
    icon_cache: RefCell<HashMap<PathBuf, DockIcon>>,
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
    ) -> Self {
        Self {
            content,
            platform,
            windows,
            apps,
            hovered,
            pressed,
            pointer,
            running_apps,
            icon_cache: RefCell::new(HashMap::new()),
        }
    }

    fn dock_rect(&self, bounds: Rect) -> Option<Rect> {
        let count = self.apps.get().len();

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

        let apps = self.apps.get();

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

    fn launch(&self, index: usize) {
        let apps = self.apps.get();

        let Some(app) = apps.get(index).cloned() else {
            return;
        };

        let process_id = match self.platform.borrow_mut().launch_app(&app) {
            Ok(process_id) => process_id,

            Err(error) => {
                eprintln!("failed to launch app {}: {error:?}", app.bundle_id,);

                return;
            }
        };

        self.windows.update(|desktop| {
            desktop.activate_process(process_id);
        });
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
        if let Some(icon) = &app.icon {
            if let DockIcon::Image(image) = self.load_icon(icon) {
                Image::new(image)
                    .content_mode(ImageContentMode::Fit)
                    .radius(CornerRadius::Custom(10.0))
                    .sampling(ImageSampling::Nearest)
                    .paint(bounds, context);

                return;
            }
        }

        let fallback = PathBuf::from(FALLBACK_APP_ICON);
        if let DockIcon::Image(image) = self.load_icon(&fallback) {
            Image::new(image)
                .content_mode(ImageContentMode::Fit)
                .radius(CornerRadius::Custom(10.0))
                .sampling(ImageSampling::Nearest)
                .paint(bounds, context);
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
            .color(RectangleColor::Custom(DOCK_TOOLTIP_BACKGROUND))
            .radius(CornerRadius::Custom(DOCK_TOOLTIP_RADIUS))
            .paint(tooltip, context);

        Text::new(app.name.clone())
            .font_size(13.0)
            .line_height(DOCK_TOOLTIP_HEIGHT)
            .alignment(TextAlignment::Center)
            .color(DOCK_TOOLTIP_TEXT)
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
            .color(EllipseColor::Custom(Color::rgba(45, 45, 45, 210)))
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
            .color(RectangleColor::Custom(DOCK_BACKGROUND))
            .radius(CornerRadius::Custom(DOCK_RADIUS))
            .shadow(ShadowStyle::Custom(DOCK_SHADOW))
            .paint(dock, context);

        Rectangle::new()
            .color(RectangleColor::Custom(Color::TRANSPARENT))
            .radius(CornerRadius::Custom(DOCK_RADIUS))
            .border(BorderStyle::custom(DOCK_BORDER, 1.0))
            .paint(dock, context);

        let apps = self.apps.get();

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
                    .color(RectangleColor::Custom(Color::rgba(
                        255,
                        255,
                        255,
                        opacity as u8,
                    )))
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

        if let Some((app, icon)) = tooltip {
            Self::paint_tooltip(app, icon, context);
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

                    self.request_dock_redraw(bounds, hit, None, context);

                    return EventResult::Consumed;
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

                    if pressed == hit {
                        if let Some(index) = hit {
                            self.launch(index);
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
