use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::platform::{AppInfo, DesktopPlatform, UserNotification};
use viewkit::{
    animation::{Animation, Easing, interpolate},
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
const BANNER_WIDTH: f32 = 370.0;
const BANNER_HEIGHT: f32 = 92.0;
const BANNER_MARGIN: f32 = 14.0;
const BANNER_TOP: f32 = 50.0;
const BANNER_ENTER_DURATION: Duration = Duration::from_millis(360);
const BANNER_VISIBLE_DURATION: Duration = Duration::from_secs(5);
const BANNER_EXIT_DURATION: Duration = Duration::from_millis(240);

#[derive(Clone, Copy, Debug)]
struct NotificationBanner {
    id: u64,
    presented_at: Instant,
    expires_at: Instant,
    dismiss_started_at: Option<Instant>,
    paused_remaining: Option<Duration>,
}

impl NotificationBanner {
    fn new(id: u64, now: Instant) -> Self {
        Self {
            id,
            presented_at: now,
            expires_at: now + BANNER_ENTER_DURATION + BANNER_VISIBLE_DURATION,
            dismiss_started_at: None,
            paused_remaining: None,
        }
    }
}

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
    banner_initialized: Cell<bool>,
    known_notification_ids: RefCell<HashSet<u64>>,
    pending_banners: RefCell<VecDeque<u64>>,
    banner: RefCell<Option<NotificationBanner>>,
    banner_hovered: Cell<bool>,
    banner_pressed: Cell<Option<u64>>,
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
            banner_initialized: Cell::new(false),
            known_notification_ids: RefCell::new(HashSet::new()),
            pending_banners: RefCell::new(VecDeque::new()),
            banner: RefCell::new(None),
            banner_hovered: Cell::new(false),
            banner_pressed: Cell::new(None),
        }
    }

    fn banner_damage(bounds: Rect) -> Rect {
        Rect::new(
            (bounds.origin.x + bounds.size.width - BANNER_WIDTH - BANNER_MARGIN - 20.0)
                .max(bounds.origin.x),
            bounds.origin.y,
            (BANNER_WIDTH + BANNER_MARGIN + 20.0).min(bounds.size.width),
            BANNER_TOP + BANNER_HEIGHT + 24.0,
        )
    }

    fn banner_target_rect(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x + (bounds.size.width - BANNER_WIDTH - BANNER_MARGIN).max(BANNER_MARGIN),
            bounds.origin.y + BANNER_TOP,
            BANNER_WIDTH.min(bounds.size.width - BANNER_MARGIN * 2.0),
            BANNER_HEIGHT,
        )
    }

    fn banner_rect(
        bounds: Rect,
        banner: NotificationBanner,
        now: Instant,
    ) -> (Rect, Option<Instant>) {
        let target = Self::banner_target_rect(bounds);
        let hidden_y = bounds.origin.y - target.size.height - BANNER_MARGIN;
        let animation = if let Some(started_at) = banner.dismiss_started_at {
            Animation::new(started_at, BANNER_EXIT_DURATION).easing(Easing::EaseInOutCubic)
        } else {
            Animation::new(banner.presented_at, BANNER_ENTER_DURATION).easing(Easing::EaseOutCubic)
        };
        let sample = animation.sample(now);
        let y = if banner.dismiss_started_at.is_some() {
            interpolate(target.origin.y, hidden_y, sample.progress)
        } else {
            interpolate(hidden_y, target.origin.y, sample.progress)
        };
        (
            Rect::new(target.origin.x, y, target.size.width, target.size.height),
            sample.next_redraw_at,
        )
    }

    fn synchronize_banners(&self, notifications: &[UserNotification], now: Instant) {
        let mut known = self.known_notification_ids.borrow_mut();
        if !self.banner_initialized.replace(true) {
            known.extend(notifications.iter().map(|notification| notification.id));
            return;
        }

        let mut pending = self.pending_banners.borrow_mut();
        for notification in notifications.iter().rev() {
            if known.insert(notification.id) {
                pending.push_back(notification.id);
            }
        }
        drop(pending);
        drop(known);

        if self.open.get() {
            self.banner.replace(None);
            self.pending_banners.borrow_mut().clear();
            self.banner_hovered.set(false);
            self.banner_pressed.set(None);
            return;
        }

        if self
            .banner
            .borrow()
            .is_some_and(|banner| !notifications.iter().any(|item| item.id == banner.id))
        {
            self.banner.replace(None);
        }

        if self.banner.borrow().is_none() {
            while let Some(id) = self.pending_banners.borrow_mut().pop_front() {
                if notifications
                    .iter()
                    .any(|notification| notification.id == id)
                {
                    self.banner.replace(Some(NotificationBanner::new(id, now)));
                    break;
                }
            }
        }
    }

    fn advance_banner(&self, notifications: &[UserNotification], now: Instant) {
        let mut banner = self.banner.borrow_mut();
        let Some(active) = banner.as_mut() else {
            return;
        };
        if active.dismiss_started_at.is_none()
            && active.paused_remaining.is_none()
            && now >= active.expires_at
        {
            active.dismiss_started_at = Some(now);
        }
        if active.dismiss_started_at.is_some_and(|started_at| {
            now.saturating_duration_since(started_at) >= BANNER_EXIT_DURATION
        }) {
            *banner = None;
            self.banner_hovered.set(false);
            self.banner_pressed.set(None);
        }
        drop(banner);

        if self.banner.borrow().is_none() {
            self.synchronize_banners(notifications, now);
        }
    }

    fn set_banner_hovered(&self, hovered: bool, now: Instant) {
        if self.banner_hovered.replace(hovered) == hovered {
            return;
        }
        let mut banner = self.banner.borrow_mut();
        let Some(active) = banner.as_mut() else {
            return;
        };
        if active.dismiss_started_at.is_some() {
            return;
        }
        if hovered {
            active.paused_remaining = Some(active.expires_at.saturating_duration_since(now));
        } else if let Some(remaining) = active.paused_remaining.take() {
            active.expires_at = now + remaining;
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

    fn paint_banner(
        &self,
        bounds: Rect,
        notifications: &[UserNotification],
        apps: &[AppInfo],
        now: Instant,
        context: &mut PaintContext<'_>,
    ) {
        let Some(banner) = *self.banner.borrow() else {
            return;
        };
        let Some(notification) = notifications
            .iter()
            .find(|notification| notification.id == banner.id)
        else {
            return;
        };
        let (card, next_animation_frame) = Self::banner_rect(bounds, banner, now);
        if let Some(next) = next_animation_frame {
            context.request_redraw_in_at(Self::banner_damage(bounds), next);
        } else if banner.dismiss_started_at.is_none()
            && let Some(next) = banner
                .paused_remaining
                .is_none()
                .then_some(banner.expires_at)
        {
            context.request_redraw_in_at(Self::banner_damage(bounds), next);
        }

        let theme = Theme::current();
        Rectangle::new()
            .color(RectangleColor::Custom(if self.banner_hovered.get() {
                theme.colors.elevated_surface
            } else {
                theme.shell.panel_background
            }))
            .radius(CornerRadius::Custom(22.0))
            .shadow(ShadowStyle::Floating)
            .paint(card, context);

        let icon_bounds = Rect::new(card.origin.x + 14.0, card.origin.y + 14.0, 44.0, 44.0);
        let app = self.app(notification, apps);
        let painted_icon = app
            .and_then(|application| application.icon.as_deref())
            .and_then(|path| self.load_icon(path))
            .map(|icon| {
                Image::new(icon)
                    .content_mode(ImageContentMode::Fit)
                    .sampling(ImageSampling::Bicubic)
                    .radius(CornerRadius::Medium)
                    .paint(icon_bounds, context);
            })
            .is_some();
        if !painted_icon {
            super::app_library::paint_fallback_icon(icon_bounds, context);
        }

        let app_name = app
            .map(|application| application.name.as_str())
            .unwrap_or(notification.bundle_id.as_str());
        Text::styled(app_name, TextRole::Caption)
            .weight(650)
            .color(theme.shell.secondary_text)
            .paint(
                Rect::new(
                    card.origin.x + 70.0,
                    card.origin.y + 10.0,
                    card.size.width - 86.0,
                    19.0,
                ),
                context,
            );
        Text::styled(&notification.title, TextRole::Label)
            .weight(700)
            .color(theme.shell.primary_text)
            .paint(
                Rect::new(
                    card.origin.x + 70.0,
                    card.origin.y + 30.0,
                    card.size.width - 86.0,
                    22.0,
                ),
                context,
            );
        Text::styled(&notification.body, TextRole::Caption)
            .color(theme.shell.secondary_text)
            .paint(
                Rect::new(
                    card.origin.x + 70.0,
                    card.origin.y + 54.0,
                    card.size.width - 86.0,
                    30.0,
                ),
                context,
            );
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
        let notifications = self.platform.borrow().notifications();
        let now = Instant::now();
        let previous_banner = self.banner.borrow().map(|banner| banner.id);
        self.synchronize_banners(&notifications, now);
        self.advance_banner(&notifications, now);
        let current_banner = self.banner.borrow().map(|banner| banner.id);
        if current_banner.is_some() && current_banner != previous_banner {
            // Notification discovery normally happens during the one-pixel platform polling
            // frame. That frame cannot paint the newly-created banner because its dirty region
            // was fixed before this layer ran, so explicitly enqueue an immediate banner frame.
            // Without it, the first visible frame can arrive after the entry transition ended.
            context.request_redraw_in_at(Self::banner_damage(bounds), now);
        }
        if !self.open.get() {
            let apps = self.apps.get();
            self.paint_banner(bounds, &notifications, &apps, now, context);
            return;
        }
        let panel = Self::panel(bounds);
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
            let now = Instant::now();
            let banner = *self.banner.borrow();
            let banner_hit = banner.map(|banner| {
                let (rect, _) = Self::banner_rect(bounds, banner, now);
                (banner.id, rect)
            });
            match event {
                ViewEvent::PointerMoved { position } => {
                    let hovered = banner_hit.is_some_and(|(_, rect)| rect.contains(*position));
                    let changed = self.banner_hovered.get() != hovered;
                    self.set_banner_hovered(hovered, now);
                    if changed {
                        context.request_redraw_in(Self::banner_damage(bounds));
                    }
                    if hovered {
                        return EventResult::Consumed;
                    }
                }
                ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                } => {
                    let pressed = banner_hit
                        .filter(|(_, rect)| rect.contains(*position))
                        .map(|(id, _)| id);
                    self.banner_pressed.set(pressed);
                    if pressed.is_some() {
                        context.request_redraw_in(Self::banner_damage(bounds));
                        return EventResult::Consumed;
                    }
                }
                ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                } => {
                    let released = banner_hit
                        .filter(|(_, rect)| rect.contains(*position))
                        .map(|(id, _)| id);
                    let pressed = self.banner_pressed.replace(None);
                    if pressed.is_some() && pressed == released {
                        self.banner.replace(None);
                        self.banner_hovered.set(false);
                        if let Some(id) = released {
                            self.activate(id);
                        }
                        context.request_redraw_in(Self::banner_damage(bounds));
                        return EventResult::Consumed;
                    }
                }
                ViewEvent::KeyPressed {
                    key: Key::Escape, ..
                } if banner.is_some() => {
                    if let Some(active) = self.banner.borrow_mut().as_mut()
                        && active.dismiss_started_at.is_none()
                    {
                        active.dismiss_started_at = Some(now);
                        active.paused_remaining = None;
                    }
                    self.banner_hovered.set(false);
                    context.request_redraw_in(Self::banner_damage(bounds));
                    return EventResult::Consumed;
                }
                _ => {}
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_enters_from_above_and_returns_above_on_dismiss() {
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let started_at = Instant::now();
        let mut banner = NotificationBanner::new(7, started_at);

        let (hidden, _) =
            NotificationCenterLayer::<Rectangle>::banner_rect(bounds, banner, started_at);
        let (visible, _) = NotificationCenterLayer::<Rectangle>::banner_rect(
            bounds,
            banner,
            started_at + BANNER_ENTER_DURATION,
        );
        assert!(hidden.origin.y < bounds.origin.y);
        assert_eq!(visible.origin.y, bounds.origin.y + BANNER_TOP);

        let dismiss_started_at = started_at + BANNER_ENTER_DURATION;
        banner.dismiss_started_at = Some(dismiss_started_at);
        let (dismissed, _) = NotificationCenterLayer::<Rectangle>::banner_rect(
            bounds,
            banner,
            dismiss_started_at + BANNER_EXIT_DURATION,
        );
        assert!(dismissed.origin.y < bounds.origin.y);
    }

    #[test]
    fn banner_stays_visible_for_five_seconds_after_entry() {
        let started_at = Instant::now();
        let banner = NotificationBanner::new(9, started_at);
        assert_eq!(
            banner.expires_at.saturating_duration_since(started_at),
            BANNER_ENTER_DURATION + BANNER_VISIBLE_DURATION
        );
    }
}
