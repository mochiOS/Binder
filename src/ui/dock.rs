use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform};

use crate::window::DesktopWindows;
use viewkit::{
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

const DOCK_TOOLTIP_HEIGHT: f32 = 30.0;
const DOCK_TOOLTIP_MARGIN: f32 = 10.0;
const DOCK_TOOLTIP_HORIZONTAL_PADDING: f32 = 12.0;
const DOCK_TOOLTIP_RADIUS: f32 = 12.0;
const DOCK_TOOLTIP_BACKGROUND: Color = Color::rgba(38, 38, 38, 230);
const DOCK_TOOLTIP_TEXT: Color = Color::rgba(255, 255, 255, 255);

const DOCK_BACKGROUND: Color = Color::rgba(255, 255, 255, 190);

const DOCK_BORDER: Color = Color::rgba(0, 0, 0, 28);

const FALLBACK_APP_ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/appicon.svg",);

const DOCK_SHADOW: ShadowSet =
    ShadowSet::single(Shadow::new(Color::rgba(0, 0, 0, 18), 0.0, 4.0, 12.0, 0.0));

struct DockItemVisual {
    item: Rect,
    icon: Rect,
    influence: f32,
}

pub(crate) struct DockLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    hovered: State<Option<usize>>,
    pressed: State<Option<usize>>,
    pointer: State<Option<Point>>,
    running_apps: State<Vec<String>>,
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
        hovered: State<Option<usize>>,
        pressed: State<Option<usize>>,
        pointer: State<Option<Point>>,
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

    fn paint_app_icon(app: &AppInfo, bounds: Rect, context: &mut PaintContext<'_>) {
        if let Some(icon) = &app.icon {
            if icon.exists() {
                if icon.extension().and_then(|extension| extension.to_str()) == Some("svg") {
                    if let Ok(svg) = SvgData::from_path(icon) {
                        Svg::new(svg)
                            .content_mode(SvgContentMode::Fit)
                            .radius(CornerRadius::Custom(10.0))
                            .paint(bounds, context);

                        return;
                    }
                }

                if let Ok(image) = ImageData::from_path(icon) {
                    Image::new(image)
                        .content_mode(ImageContentMode::Fit)
                        .radius(CornerRadius::Custom(10.0))
                        .paint(bounds, context);

                    return;
                }
            }
        }

        let fallback = PathBuf::from(FALLBACK_APP_ICON);

        if let Ok(svg) = SvgData::from_path(fallback) {
            Svg::new(svg)
                .content_mode(SvgContentMode::Fit)
                .radius(CornerRadius::Custom(10.0))
                .paint(bounds, context);
        }
    }

    fn tooltip_width(text: &str) -> f32 {
        let character_count = text.chars().count() as f32;

        let text_width = character_count * 7.5;

        (text_width + DOCK_TOOLTIP_HORIZONTAL_PADDING * 2.0).clamp(48.0, 220.0)
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

                Rectangle::new()
                    .color(RectangleColor::Custom(Color::rgba(
                        255,
                        255,
                        255,
                        opacity as u8,
                    )))
                    .radius(CornerRadius::Custom(DOCK_ITEM_RADIUS))
                    .paint(visual.item, context);
            }

            let icon = snap_rect(visual.icon);

            Self::paint_app_icon(app, icon, context);

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

                let changed =
                    self.hovered.get() != hit || self.pointer.get() != inside.then_some(*position);

                self.hovered.set(hit);

                if inside {
                    self.pointer.set(Some(*position));
                } else {
                    self.pointer.set(None);
                }

                if changed {
                    context.request_redraw();
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
                    self.pressed.set(self.hit_index(bounds, *position));

                    context.request_redraw();

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

                    context.request_redraw();

                    return EventResult::Consumed;
                }

                if self.is_inside_dock(bounds, *position) {
                    return EventResult::Consumed;
                }

                self.content.handle_event(bounds, event, context)
            }

            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                let changed = self.hovered.get().is_some()
                    || self.pressed.get().is_some()
                    || self.pointer.get().is_some();

                self.hovered.set(None);
                self.pressed.set(None);
                self.pointer.set(None);

                if changed {
                    context.request_redraw();
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
