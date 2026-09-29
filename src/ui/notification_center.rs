use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform, UserNotification};
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const PANEL_WIDTH: f32 = 390.0;
const PANEL_TOP: f32 = 47.0;
const PANEL_MARGIN: f32 = 14.0;
const PANEL_PADDING: f32 = 16.0;
const HEADER_HEIGHT: f32 = 42.0;
const ROW_HEIGHT: f32 = 102.0;
const FALLBACK_ICON: &str = "/applications/Binder.app/appicon.svg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Clear,
    Remove(u64),
    Notification(u64),
}

pub(crate) struct NotificationCenterLayer<C> {
    content: C,
    open: State<bool>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    apps: State<Vec<AppInfo>>,
    hovered: Cell<Option<Hit>>,
    pressed: Cell<Option<Hit>>,
    scroll: Cell<usize>,
    icon_cache: RefCell<HashMap<PathBuf, ImageData>>,
}

impl<C: View> NotificationCenterLayer<C> {
    pub(crate) fn new(
        content: C,
        open: State<bool>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        apps: State<Vec<AppInfo>>,
    ) -> Self {
        Self {
            content,
            open,
            platform,
            apps,
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            scroll: Cell::new(0),
            icon_cache: RefCell::new(HashMap::new()),
        }
    }

    fn panel(bounds: Rect) -> Rect {
        let height = (bounds.size.height - PANEL_TOP - PANEL_MARGIN).clamp(240.0, 620.0);
        Rect::new(
            bounds.origin.x + (bounds.size.width - PANEL_WIDTH - PANEL_MARGIN).max(PANEL_MARGIN),
            bounds.origin.y + PANEL_TOP,
            PANEL_WIDTH.min(bounds.size.width - PANEL_MARGIN * 2.0),
            height,
        )
    }

    fn clear_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + panel.size.width - 92.0,
            panel.origin.y + 10.0,
            72.0,
            28.0,
        )
    }

    fn visible_count(panel: Rect) -> usize {
        ((panel.size.height - HEADER_HEIGHT - PANEL_PADDING) / ROW_HEIGHT)
            .floor()
            .max(1.0) as usize
    }

    fn row_rect(panel: Rect, visible_index: usize) -> Rect {
        Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + HEADER_HEIGHT + visible_index as f32 * ROW_HEIGHT,
            panel.size.width - PANEL_PADDING * 2.0,
            ROW_HEIGHT - 10.0,
        )
    }

    fn remove_rect(row: Rect) -> Rect {
        Rect::new(
            row.origin.x + row.size.width - 32.0,
            row.origin.y + 8.0,
            24.0,
            24.0,
        )
    }

    fn hit(
        &self,
        bounds: Rect,
        position: Point,
        notifications: &[UserNotification],
    ) -> Option<Hit> {
        let panel = Self::panel(bounds);
        if !panel.contains(position) {
            return None;
        }
        if Self::clear_rect(panel).contains(position) && !notifications.is_empty() {
            return Some(Hit::Clear);
        }
        let start = self.scroll.get();
        for (visible_index, notification) in notifications
            .iter()
            .skip(start)
            .take(Self::visible_count(panel))
            .enumerate()
        {
            let row = Self::row_rect(panel, visible_index);
            if Self::remove_rect(row).contains(position) {
                return Some(Hit::Remove(notification.id));
            }
            if row.contains(position) {
                return Some(Hit::Notification(notification.id));
            }
        }
        None
    }

    fn app<'a>(&self, notification: &UserNotification, apps: &'a [AppInfo]) -> Option<&'a AppInfo> {
        apps.iter()
            .find(|application| application.bundle_id == notification.bundle_id)
    }

    fn load_icon(&self, path: &Path) -> Option<ImageData> {
        if let Some(icon) = self.icon_cache.borrow().get(path) {
            return Some(icon.clone());
        }
        let icon = if path.extension().and_then(|extension| extension.to_str()) == Some("svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| ImageData::from_svg(&svg, 72, 72).ok())
        } else {
            ImageData::thumbnail_from_path(path, 72, 72).ok()
        }?;
        self.icon_cache
            .borrow_mut()
            .insert(path.to_path_buf(), icon.clone());
        Some(icon)
    }

    fn paint_notification(
        &self,
        notification: &UserNotification,
        app: Option<&AppInfo>,
        row: Rect,
        context: &mut PaintContext<'_>,
    ) {
        let theme = Theme::current();
        let hovered = matches!(
            self.hovered.get(),
            Some(Hit::Notification(id) | Hit::Remove(id)) if id == notification.id
        );
        Rectangle::new()
            .color(RectangleColor::Custom(if hovered {
                theme.shell.item_hover
            } else if notification.read {
                theme.shell.item_enabled
            } else {
                theme.shell.selection_soft
            }))
            .radius(CornerRadius::Custom(16.0))
            .paint(row, context);

        let icon_bounds = Rect::new(row.origin.x + 12.0, row.origin.y + 13.0, 38.0, 38.0);
        let icon_path = app
            .and_then(|application| application.icon.as_deref())
            .unwrap_or_else(|| Path::new(FALLBACK_ICON));
        if let Some(icon) = self.load_icon(icon_path) {
            Image::new(icon)
                .content_mode(ImageContentMode::Fit)
                .sampling(ImageSampling::Bicubic)
                .radius(CornerRadius::Medium)
                .paint(icon_bounds, context);
        }

        let app_name = app
            .map(|application| application.name.as_str())
            .unwrap_or(notification.bundle_id.as_str());
        Text::styled(app_name, TextRole::Caption)
            .weight(650)
            .color(theme.shell.secondary_text)
            .paint(
                Rect::new(
                    row.origin.x + 60.0,
                    row.origin.y + 8.0,
                    row.size.width - 104.0,
                    20.0,
                ),
                context,
            );
        Text::styled(&notification.title, TextRole::Label)
            .weight(650)
            .color(theme.shell.primary_text)
            .paint(
                Rect::new(
                    row.origin.x + 60.0,
                    row.origin.y + 29.0,
                    row.size.width - 76.0,
                    22.0,
                ),
                context,
            );
        Text::styled(&notification.body, TextRole::Caption)
            .color(theme.shell.secondary_text)
            .paint(
                Rect::new(
                    row.origin.x + 60.0,
                    row.origin.y + 52.0,
                    row.size.width - 76.0,
                    32.0,
                ),
                context,
            );

        let remove = Self::remove_rect(row);
        if self.hovered.get() == Some(Hit::Remove(notification.id)) {
            Rectangle::new()
                .color(RectangleColor::Custom(theme.shell.item_hover))
                .radius(CornerRadius::Custom(12.0))
                .paint(remove, context);
        }
        Icon::new(SymbolName::X)
            .size(9.0)
            .color(theme.shell.secondary_text)
            .paint(remove, context);
    }

    fn activate(&self, id: u64) {
        let notification = self
            .platform
            .borrow()
            .notifications()
            .into_iter()
            .find(|notification| notification.id == id);
        let Some(notification) = notification else {
            return;
        };
        let app = self
            .apps
            .get()
            .into_iter()
            .find(|application| application.bundle_id == notification.bundle_id);
        let Some(app) = app else {
            return;
        };
        let mut platform = self.platform.borrow_mut();
        if let Some(process) = platform.process_id_for_bundle(&app.bundle_id) {
            let _ = platform.activate_application(process);
        } else {
            let _ = platform.launch_app(&app);
        }
        self.open.set(false);
    }
}

impl<C: View> View for NotificationCenterLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        if !self.open.get() {
            return;
        }
        let panel = Self::panel(bounds);
        let notifications = self.platform.borrow().notifications();
        let apps = self.apps.get();
        let maximum = notifications
            .len()
            .saturating_sub(Self::visible_count(panel));
        self.scroll.set(self.scroll.get().min(maximum));

        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.panel_background,
            ))
            .radius(CornerRadius::Custom(22.0))
            .shadow(ShadowStyle::Card)
            .paint(panel, context);
        Text::styled("Notifications", TextRole::TitleSmall)
            .weight(700)
            .color(Theme::current().shell.primary_text)
            .paint(
                Rect::new(
                    panel.origin.x + PANEL_PADDING,
                    panel.origin.y + 12.0,
                    220.0,
                    26.0,
                ),
                context,
            );
        if !notifications.is_empty() {
            let clear = Self::clear_rect(panel);
            if self.hovered.get() == Some(Hit::Clear) {
                Rectangle::new()
                    .color(RectangleColor::Custom(Theme::current().shell.item_hover))
                    .radius(CornerRadius::Custom(14.0))
                    .paint(clear, context);
            }
            Text::styled("Clear All", TextRole::Caption)
                .weight(650)
                .alignment(TextAlignment::Center)
                .color(Theme::current().colors.accent)
                .paint(clear, context);
        }
        if notifications.is_empty() {
            Icon::new(SymbolName::Bell)
                .size(28.0)
                .color(Theme::current().shell.tertiary_text)
                .paint(
                    Rect::new(
                        panel.origin.x,
                        panel.origin.y + panel.size.height * 0.38,
                        panel.size.width,
                        36.0,
                    ),
                    context,
                );
            Text::styled("No Notifications", TextRole::Label)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(
                        panel.origin.x,
                        panel.origin.y + panel.size.height * 0.48,
                        panel.size.width,
                        24.0,
                    ),
                    context,
                );
            return;
        }
        for (visible_index, notification) in notifications
            .iter()
            .skip(self.scroll.get())
            .take(Self::visible_count(panel))
            .enumerate()
        {
            self.paint_notification(
                notification,
                self.app(notification, &apps),
                Self::row_rect(panel, visible_index),
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
        if !self.open.get() {
            return self.content.handle_event(bounds, event, context);
        }
        let notifications = self.platform.borrow().notifications();
        let panel = Self::panel(bounds);
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                self.open.set(false);
                self.hovered.set(None);
                context.request_redraw();
            }
            ViewEvent::PointerMoved { position } => {
                let hit = self.hit(bounds, *position, &notifications);
                if self.hovered.replace(hit) != hit {
                    context.request_redraw_in(panel);
                }
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if !panel.contains(*position) {
                    self.open.set(false);
                }
                self.pressed
                    .set(self.hit(bounds, *position, &notifications));
                context.request_redraw();
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let released = self.hit(bounds, *position, &notifications);
                let pressed = self.pressed.replace(None);
                if pressed == released {
                    match released {
                        Some(Hit::Clear) => {
                            let _ = self.platform.borrow_mut().clear_notifications();
                            self.scroll.set(0);
                        }
                        Some(Hit::Remove(id)) => {
                            let _ = self.platform.borrow_mut().remove_notification(id);
                        }
                        Some(Hit::Notification(id)) => self.activate(id),
                        None => {}
                    }
                }
                context.request_redraw();
            }
            ViewEvent::Scroll {
                position, delta_y, ..
            } if panel.contains(*position) => {
                let maximum = notifications
                    .len()
                    .saturating_sub(Self::visible_count(panel));
                let next = if *delta_y < 0.0 {
                    self.scroll.get().saturating_add(1).min(maximum)
                } else {
                    self.scroll.get().saturating_sub(1)
                };
                if self.scroll.replace(next) != next {
                    context.request_redraw_in(panel);
                }
            }
            _ => {}
        }
        EventResult::Consumed
    }
}
