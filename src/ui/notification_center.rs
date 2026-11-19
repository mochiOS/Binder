use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};
#[cfg(not(target_os = "mochios"))]
use std::time::{SystemTime, UNIX_EPOCH};

use crate::platform::{AppInfo, DesktopPlatform, UserNotification};
use viewkit::{
    animation::{Animation, Easing, interpolate},
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

#[derive(Default)]
pub(crate) struct NotificationCenterInteraction {
    banner_initialized: Cell<bool>,
    known_notification_ids: RefCell<HashSet<u64>>,
    pending_banners: RefCell<VecDeque<u64>>,
    banner: RefCell<Option<NotificationBanner>>,
    banner_hovered: Cell<bool>,
    banner_pressed: Cell<Option<u64>>,
    panel_motion: RefCell<VisibilityTransition>,
    expanded_groups: RefCell<HashSet<String>>,
    scroll: Cell<f32>,
    controls: RefCell<HashMap<Hit, ButtonInteractionState>>,
    was_open: Cell<bool>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Hit {
    Clear,
    Remove(u64),
    Notification(u64),
    Group(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CenterEntry {
    Group {
        latest: u64,
        bundle: String,
        count: usize,
    },
    Notification(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CenterPresentation {
    Panel,
    Tablet,
}

impl CenterEntry {
    fn height(&self, theme: &Theme, presentation: CenterPresentation) -> f32 {
        if presentation == CenterPresentation::Tablet {
            return match self {
                Self::Group { .. } => theme.controls.touch_height + theme.spacing.small,
                Self::Notification(_) => {
                    theme.spacing.medium * 2.0
                        + theme.typography.caption.line_height
                        + theme.typography.label.line_height
                        + theme.typography.body.line_height * 2.0
                        + theme.spacing.extra_small * 2.0
                        + theme.spacing.small
                }
            };
        }
        match self {
            Self::Group { .. } => theme.layout.control_height + theme.spacing.small,
            Self::Notification(_) => ROW_HEIGHT,
        }
    }
}

fn grouped_entries(
    notifications: &[UserNotification],
    expanded: &HashSet<String>,
) -> Vec<CenterEntry> {
    // The service snapshot is newest-first. Its delivery order also remains
    // valid across boots, unlike the native service's boot-relative ticks.
    let mut groups: Vec<(&str, Vec<u64>)> = Vec::new();
    for item in notifications {
        if let Some((_, ids)) = groups
            .iter_mut()
            .find(|(bundle, _)| *bundle == item.bundle_id)
        {
            ids.push(item.id);
        } else {
            groups.push((&item.bundle_id, vec![item.id]));
        }
    }
    let mut rows = Vec::new();
    for (bundle, ids) in groups {
        if ids.len() > 1 {
            rows.push(CenterEntry::Group {
                latest: ids[0],
                bundle: bundle.to_owned(),
                count: ids.len(),
            });
        }
        let count = if expanded.contains(bundle) {
            ids.len()
        } else {
            1
        };
        rows.extend(ids.into_iter().take(count).map(CenterEntry::Notification));
    }
    rows
}

pub(crate) struct NotificationCenterLayer<C> {
    presentation: CenterPresentation,
    content: C,
    open: State<bool>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    apps: State<Vec<AppInfo>>,
    hovered: Cell<Option<Hit>>,
    pressed: Cell<Option<Hit>>,
    icon_cache: super::app_icon::AppIcons,
    interaction: Rc<NotificationCenterInteraction>,
    activation: Rc<RefCell<super::app_library::PendingAppActivation>>,
}

impl<C: View> NotificationCenterLayer<C> {
    pub(crate) fn new(
        content: C,
        open: State<bool>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        apps: State<Vec<AppInfo>>,
        interaction: Rc<NotificationCenterInteraction>,
        activation: Rc<RefCell<super::app_library::PendingAppActivation>>,
    ) -> Self {
        Self {
            presentation: CenterPresentation::Panel,
            content,
            open,
            platform,
            apps,
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            icon_cache: super::app_icon::AppIcons::default(),
            interaction,
            activation,
        }
    }

    pub(super) fn with_tablet_presentation(mut self) -> Self {
        self.presentation = CenterPresentation::Tablet;
        self
    }

    fn is_tablet(&self) -> bool {
        self.presentation == CenterPresentation::Tablet
    }

    fn frame(&self, bounds: Rect) -> Rect {
        if self.is_tablet() {
            bounds
        } else {
            Self::panel(bounds)
        }
    }

    pub(super) fn scroll_bounds(&self, bounds: Rect) -> Rect {
        self.body_rect(self.frame(bounds))
    }

    pub(super) fn scroll_offset(&self) -> f32 {
        self.interaction.scroll.get()
    }

    pub(super) fn set_scroll_offset(&self, bounds: Rect, offset: f32) -> f32 {
        let notifications = self.platform.borrow().notifications();
        let maximum = self.maximum_scroll(self.frame(bounds), &self.entries(&notifications));
        let offset = offset.clamp(0.0, maximum);
        self.interaction.scroll.set(offset);
        offset
    }

    pub(super) fn banner_contains(&self, bounds: Rect, position: Point) -> bool {
        self.interaction
            .banner
            .borrow()
            .as_ref()
            .is_some_and(|banner| {
                Self::banner_rect(bounds, *banner, Instant::now())
                    .0
                    .contains(position)
            })
    }

    fn banner_damage(bounds: Rect) -> Rect {
        let extent = Self::floating_extent();
        Rect::new(
            (bounds.origin.x + bounds.size.width - BANNER_WIDTH - BANNER_MARGIN - extent)
                .max(bounds.origin.x),
            bounds.origin.y,
            (BANNER_WIDTH + BANNER_MARGIN + extent).min(bounds.size.width),
            BANNER_TOP + BANNER_HEIGHT + extent,
        )
    }

    fn floating_extent() -> f32 {
        let theme = Theme::current();
        theme.surface.shadow_outset(&theme.shadows)
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

    fn synchronize_banners(
        &self,
        notifications: &[UserNotification],
        focus_enabled: bool,
        now: Instant,
    ) {
        let mut known = self.interaction.known_notification_ids.borrow_mut();
        if !self.interaction.banner_initialized.replace(true) {
            known.extend(notifications.iter().map(|notification| notification.id));
            return;
        }
        known.retain(|id| notifications.iter().any(|item| item.id == *id));

        let mut pending = self.interaction.pending_banners.borrow_mut();
        for notification in notifications.iter().rev() {
            if known.insert(notification.id) && !focus_enabled {
                pending.push_back(notification.id);
            }
        }
        drop(pending);
        drop(known);

        if self.open.get() || focus_enabled {
            self.interaction.banner.replace(None);
            self.interaction.pending_banners.borrow_mut().clear();
            self.interaction.banner_hovered.set(false);
            self.interaction.banner_pressed.set(None);
            return;
        }

        if self
            .interaction
            .banner
            .borrow()
            .is_some_and(|banner| !notifications.iter().any(|item| item.id == banner.id))
        {
            self.interaction.banner.replace(None);
        }

        if self.interaction.banner.borrow().is_none() {
            while let Some(id) = self.interaction.pending_banners.borrow_mut().pop_front() {
                if notifications
                    .iter()
                    .any(|notification| notification.id == id)
                {
                    self.interaction
                        .banner
                        .replace(Some(NotificationBanner::new(id, now)));
                    break;
                }
            }
        }
    }

    fn advance_banner(
        &self,
        notifications: &[UserNotification],
        focus_enabled: bool,
        now: Instant,
    ) {
        let mut banner = self.interaction.banner.borrow_mut();
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
            self.interaction.banner_hovered.set(false);
            self.interaction.banner_pressed.set(None);
        }
        drop(banner);

        if self.interaction.banner.borrow().is_none() {
            self.synchronize_banners(notifications, focus_enabled, now);
        }
    }

    fn set_banner_hovered(&self, hovered: bool, now: Instant) {
        if self.interaction.banner_hovered.replace(hovered) == hovered {
            return;
        }
        let mut banner = self.interaction.banner.borrow_mut();
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
        let height = (bounds.size.height - PANEL_TOP - PANEL_MARGIN).clamp(0.0, 620.0);
        let width = PANEL_WIDTH.min((bounds.size.width - PANEL_MARGIN * 2.0).max(0.0));
        Rect::new(
            bounds.origin.x + (bounds.size.width - width - PANEL_MARGIN).max(0.0),
            bounds.origin.y + PANEL_TOP,
            width,
            height,
        )
    }

    fn panel_damage(&self, bounds: Rect) -> Rect {
        if self.is_tablet() {
            return bounds;
        }
        let target = Self::panel(bounds);
        let extent = Self::floating_extent();
        Rect::new(
            target.origin.x - extent,
            bounds.origin.y,
            target.size.width + PANEL_MARGIN + extent,
            bounds.size.height,
        )
        .intersection(bounds)
        .unwrap_or(bounds)
    }

    fn panel_presentation(&self, bounds: Rect, now: Instant) -> (Rect, f32, Option<Instant>) {
        if self.is_tablet() {
            return (
                self.frame(bounds),
                if self.open.get() { 1.0 } else { 0.0 },
                None,
            );
        }
        let (amount, next) = self.interaction.panel_motion.borrow_mut().sample(
            self.open.get(),
            Theme::current().motion.standard,
            now,
        );
        let mut panel = Self::panel(bounds);
        panel.origin.x = interpolate(
            bounds.origin.x + bounds.size.width + PANEL_MARGIN,
            panel.origin.x,
            amount,
        );
        (panel, amount, next)
    }

    fn clear_rect(&self, panel: Rect) -> Rect {
        if self.is_tablet() {
            let size = Theme::current().controls.touch_height;
            return Rect::new(
                panel.origin.x + panel.size.width - size,
                panel.origin.y,
                size,
                size,
            );
        }
        Rect::new(
            panel.origin.x + panel.size.width - 92.0,
            panel.origin.y + 10.0,
            72.0,
            28.0,
        )
    }

    fn body_rect(&self, panel: Rect) -> Rect {
        if self.is_tablet() {
            let theme = Theme::current();
            let header = theme.controls.touch_height + theme.spacing.small;
            return Rect::new(
                panel.origin.x,
                panel.origin.y + header,
                panel.size.width,
                (panel.size.height - header).max(0.0),
            );
        }
        Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + HEADER_HEIGHT,
            (panel.size.width - PANEL_PADDING * 2.0).max(0.0),
            (panel.size.height - HEADER_HEIGHT - PANEL_PADDING).max(0.0),
        )
    }

    fn entries(&self, notifications: &[UserNotification]) -> Vec<CenterEntry> {
        grouped_entries(notifications, &self.interaction.expanded_groups.borrow())
    }

    fn maximum_scroll(&self, panel: Rect, entries: &[CenterEntry]) -> f32 {
        let theme = Theme::current();
        (entries
            .iter()
            .map(|entry| entry.height(&theme, self.presentation))
            .sum::<f32>()
            - self.body_rect(panel).size.height)
            .max(0.0)
    }

    fn visible_entries(
        &self,
        panel: Rect,
        notifications: &[UserNotification],
    ) -> Vec<(CenterEntry, Rect)> {
        let entries = self.entries(notifications);
        let theme = Theme::current();
        let offset = self
            .interaction
            .scroll
            .get()
            .clamp(0.0, self.maximum_scroll(panel, &entries));
        self.interaction.scroll.set(offset);
        let body = self.body_rect(panel);
        let mut y = body.origin.y - offset;
        entries
            .into_iter()
            .filter_map(|entry| {
                let height = entry.height(&theme, self.presentation);
                let row = Rect::new(
                    body.origin.x,
                    y,
                    body.size.width,
                    (height - theme.spacing.small).max(0.0),
                );
                y += height;
                row.intersection(body).map(|_| (entry, row))
            })
            .collect()
    }

    fn control(&self, hit: Hit, label: impl Into<String>) -> Button {
        let state = self
            .interaction
            .controls
            .borrow_mut()
            .entry(hit)
            .or_default()
            .clone();
        Button::with_interaction_and_label(state, label)
            .style(ButtonStyle::Ghost)
            .size(if self.is_tablet() {
                ButtonSize::Touch
            } else {
                ButtonSize::Small
            })
    }

    fn clear_control(&self) -> Button {
        let button = self
            .control(Hit::Clear, "Clear All")
            .accessibility_label("Clear all notifications");
        if self.is_tablet() {
            button.content_padding(false).content(
                Icon::new(SymbolName::X)
                    .size(Theme::current().layout.control_icon_size)
                    .color(Color::WHITE),
            )
        } else {
            button
        }
    }

    fn row_control(&self, entry: &CenterEntry, row: Rect, apps: &[AppInfo]) -> (Hit, Rect, Button) {
        match entry {
            CenterEntry::Group {
                latest,
                bundle,
                count,
            } => {
                let expanded = self.interaction.expanded_groups.borrow().contains(bundle);
                let name = apps
                    .iter()
                    .find(|app| app.bundle_id == *bundle)
                    .map(|app| app.name.as_str())
                    .unwrap_or(bundle);
                let label = format!("{name} ({count})");
                let hit = Hit::Group(*latest);
                let button = self
                    .control(hit, label.clone())
                    .accessibility_label(label.clone())
                    .content(
                        HStack::new()
                            .gap(StackGap::Small)
                            .child(
                                Icon::new(if expanded {
                                    SymbolName::ChevronDown
                                } else {
                                    SymbolName::ChevronRight
                                })
                                .color(if self.is_tablet() { Color::WHITE } else { Theme::current().shell.secondary_text })
                                .size(14.0),
                            )
                            .child(Text::label(label).color(if self.is_tablet() { Color::WHITE }
                                else { Theme::current().shell.primary_text }).layout().flex_grow(1.0)),
                    );
                (hit, row, button)
            }
            CenterEntry::Notification(id) => {
                let hit = Hit::Remove(*id);
                let button = self
                    .control(hit, "")
                    .accessibility_label("Dismiss notification")
                    .content_padding(false)
                    .content(Icon::new(SymbolName::X).size(if self.is_tablet() {
                        Theme::current().layout.compact_icon_size
                    } else {
                        10.0
                    }));
                (hit, self.remove_rect(row), button)
            }
        }
    }

    fn apply(&self, hit: Hit, notifications: &[UserNotification]) {
        match hit {
            Hit::Clear => {
                if self.platform.borrow_mut().clear_notifications().is_ok() {
                    self.interaction.scroll.set(0.0);
                    self.interaction.expanded_groups.borrow_mut().clear();
                }
            }
            Hit::Remove(id) => {
                let _ = self.platform.borrow_mut().remove_notification(id);
            }
            Hit::Notification(id) => self.activate(id),
            Hit::Group(id) => {
                if let Some(item) = notifications.iter().find(|item| item.id == id) {
                    let mut groups = self.interaction.expanded_groups.borrow_mut();
                    if !groups.remove(&item.bundle_id) {
                        groups.insert(item.bundle_id.clone());
                    }
                }
            }
        }
    }

    fn remove_rect(&self, row: Rect) -> Rect {
        if self.is_tablet() {
            let theme = Theme::current();
            let size = theme.controls.touch_height;
            return Rect::new(
                row.origin.x + row.size.width - size - theme.spacing.extra_small,
                row.origin.y + theme.spacing.extra_small,
                size,
                size,
            );
        }
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
        let (panel, _, _) = self.panel_presentation(bounds, Instant::now());
        if !panel.contains_rounded(
            Theme::current().surface.floating_radius.resolve(
                &Theme::current().radius,
                panel.size.width,
                panel.size.height,
            ),
            position,
        ) {
            return None;
        }
        if self.clear_rect(panel).contains(position) && !notifications.is_empty() {
            return Some(Hit::Clear);
        }
        if !self.body_rect(panel).contains(position) {
            return None;
        }
        for (entry, row) in self.visible_entries(panel, notifications) {
            match entry {
                CenterEntry::Group { latest, .. } if row.contains(position) => {
                    return Some(Hit::Group(latest));
                }
                CenterEntry::Notification(id) if row.contains(position) => {
                    return Some(if self.remove_rect(row).contains(position) {
                        Hit::Remove(id)
                    } else {
                        Hit::Notification(id)
                    });
                }
                _ => {}
            }
        }
        None
    }

    fn app<'a>(&self, notification: &UserNotification, apps: &'a [AppInfo]) -> Option<&'a AppInfo> {
        apps.iter()
            .find(|application| application.bundle_id == notification.bundle_id)
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
        let card = if self.is_tablet() {
            Card::new()
                .radius(CornerRadius::ExtraLarge)
                .border(BorderStyle::None)
        } else {
            Card::new()
        };
        card.color(RectangleColor::Custom(if hovered {
            theme.shell.item_hover
        } else {
            theme.card.background
        }))
        .shadow(ShadowStyle::None)
        .paint(row, context);

        let inset = if self.is_tablet() {
            theme.spacing.medium
        } else {
            12.0
        };
        let icon_size = if self.is_tablet() {
            theme.layout.avatar_size
        } else {
            38.0
        };
        let icon_bounds = Rect::new(
            row.origin.x + inset,
            row.origin.y + inset,
            icon_size,
            icon_size,
        );
        let text_x = if self.is_tablet() {
            icon_bounds.origin.x + icon_size + theme.spacing.small
        } else {
            row.origin.x + 60.0
        };
        let header_top = if self.is_tablet() { inset } else { 8.0 };
        let title_top = if self.is_tablet() {
            header_top + theme.typography.caption.line_height + theme.spacing.extra_small
        } else {
            29.0
        };
        let body_top = if self.is_tablet() {
            title_top + theme.typography.label.line_height + theme.spacing.extra_small
        } else {
            52.0
        };
        let text_width = (row.origin.x + row.size.width - inset - text_x).max(0.0);
        let timestamp_right = if self.is_tablet() {
            self.remove_rect(row).origin.x
        } else {
            row.origin.x + row.size.width - 42.0
        };
        if let Some(app) = app {
            self.icon_cache.paint(app, icon_bounds, context);
        } else {
            super::app_icon::paint_fallback_icon(icon_bounds, context);
        }

        let app_name = app
            .map(|application| application.name.as_str())
            .unwrap_or(notification.bundle_id.as_str());
        Text::styled(app_name, TextRole::Caption)
            .weight(650)
            .color(theme.shell.secondary_text)
            .paint(
                Rect::new(
                    text_x,
                    row.origin.y + header_top,
                    (timestamp_right - 56.0 - theme.spacing.small - text_x).max(0.0),
                    theme.typography.caption.line_height,
                ),
                context,
            );
        #[cfg(not(target_os = "mochios"))]
        let age = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(notification.created_at);
        #[cfg(not(target_os = "mochios"))]
        let timestamp = if age < 60 {
            "Now".to_owned()
        } else if age < 3600 {
            format!("{}m", age / 60)
        } else if age < 86400 {
            format!("{}h", age / 3600)
        } else {
            format!("{}d", age / 86400)
        };
        // Native workspace timestamps are boot ticks, not Unix seconds.
        #[cfg(target_os = "mochios")]
        let timestamp = String::new();
        Text::caption(timestamp)
            .alignment(TextAlignment::End)
            .color(theme.shell.tertiary_text)
            .paint(
                Rect::new(
                    timestamp_right - 56.0,
                    row.origin.y + header_top,
                    56.0,
                    theme.typography.caption.line_height,
                ),
                context,
            );
        if !notification.read {
            Ellipse::new().color(RectangleColor::Accent).paint(
                Rect::new(row.origin.x + 4.0, row.origin.y + 17.0, 4.0, 4.0),
                context,
            );
        }
        Text::styled(&notification.title, TextRole::Label)
            .weight(650)
            .color(theme.shell.primary_text)
            .paint(
                Rect::new(
                    text_x,
                    row.origin.y + title_top,
                    (text_width - if self.is_tablet() { theme.controls.touch_height } else { 0.0 }).max(0.0),
                    theme.typography.label.line_height,
                ),
                context,
            );
        Text::styled(
            &notification.body,
            if self.is_tablet() {
                TextRole::Body
            } else {
                TextRole::Caption
            },
        )
        .color(theme.shell.secondary_text)
        .paint(
            Rect::new(
                text_x,
                row.origin.y + body_top,
                text_width,
                if self.is_tablet() {
                    theme.typography.body.line_height * 2.0
                } else {
                    32.0
                },
            ),
            context,
        );
    }

    fn paint_banner(
        &self,
        bounds: Rect,
        notifications: &[UserNotification],
        apps: &[AppInfo],
        now: Instant,
        context: &mut PaintContext<'_>,
    ) {
        let Some(banner) = *self.interaction.banner.borrow() else {
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
        Surface::floating().paint(card, context);

        let icon_bounds = Rect::new(card.origin.x + 14.0, card.origin.y + 14.0, 44.0, 44.0);
        let app = self.app(notification, apps);
        let painted_icon = app
            .and_then(|application| application.icon.as_deref())
            .and_then(|path| self.icon_cache.load(path,icon_bounds,context.scale_factor()))
            .map(|icon| {
                super::app_icon::paint(icon,icon_bounds,context);
            })
            .is_some();
        if !painted_icon {
            super::app_icon::paint_fallback_icon(icon_bounds, context);
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
        self.activation.borrow_mut().queue(app);
        self.open.set(false);
    }
}

impl<C: View> View for NotificationCenterLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        if self.interaction.was_open.replace(self.open.get()) != self.open.get() && self.open.get()
        {
            let _ = self.platform.borrow_mut().mark_all_notifications_read();
        }
        let notifications = self.platform.borrow().notifications();
        let focus_enabled = self.platform.borrow().notification_settings().focus_enabled;
        let now = Instant::now();
        let previous_banner = self.interaction.banner.borrow().map(|banner| banner.id);
        self.synchronize_banners(&notifications, focus_enabled, now);
        self.advance_banner(&notifications, focus_enabled, now);
        let current_banner = self.interaction.banner.borrow().map(|banner| banner.id);
        if current_banner.is_some() && current_banner != previous_banner {
            // Notification discovery normally happens during the one-pixel platform polling
            // frame. That frame cannot paint the newly-created banner because its dirty region
            // was fixed before this layer ran, so explicitly enqueue an immediate banner frame.
            // Without it, the first visible frame can arrive after the entry transition ended.
            context.request_redraw_in_at(Self::banner_damage(bounds), now);
        }
        let (panel, amount, next_panel_frame) = self.panel_presentation(bounds, now);
        if let Some(next) = next_panel_frame {
            context.request_redraw_in_at(self.panel_damage(bounds), next);
        }
        if amount <= 0.0 && !self.open.get() {
            let apps = self.apps.get();
            self.paint_banner(bounds, &notifications, &apps, now, context);
            return;
        }
        let apps = self.apps.get();
        self.interaction
            .controls
            .borrow_mut()
            .retain(|hit, _| match hit {
                Hit::Clear => true,
                Hit::Remove(id) | Hit::Group(id) | Hit::Notification(id) => {
                    notifications.iter().any(|item| item.id == *id)
                }
            });
        self.interaction
            .expanded_groups
            .borrow_mut()
            .retain(|bundle| notifications.iter().any(|item| item.bundle_id == *bundle));
        if !self.is_tablet() {
            Surface::floating().paint(panel, context);
        }
        if self.open.get() {
            let mut scope = AccessibilityNode::new(AccessibilityRole::Group, self.frame(bounds));
            scope.label = Some("Notification Center".into());
            scope.focus_scope = true;
            context.record_accessibility(scope);
        }
        let theme = Theme::current();
        if !self.is_tablet() || !notifications.is_empty() {
            Text::styled("Notifications", TextRole::TitleSmall)
                .color(if self.is_tablet() {
                    Color::WHITE
                } else {
                    theme.shell.primary_text
                })
                .paint(
                    Rect::new(
                        panel.origin.x
                            + if self.is_tablet() {
                                theme.spacing.extra_small
                            } else {
                                PANEL_PADDING
                            },
                        panel.origin.y
                            + if self.is_tablet() {
                                (theme.controls.touch_height
                                    - theme.typography.title_small.line_height)
                                    * 0.5
                            } else {
                                12.0
                            },
                        (panel.size.width - theme.controls.touch_height - theme.spacing.small)
                            .max(0.0),
                        theme.typography.title_small.line_height,
                    ),
                    context,
                );
        }
        if !notifications.is_empty() {
            self.clear_control().paint(self.clear_rect(panel), context);
        }
        if notifications.is_empty() {
            let body = self.body_rect(panel);
            let center_y = body.origin.y + body.size.height * 0.35;
            let foreground = if self.is_tablet() {
                Color::WHITE.with_alpha(210)
            } else {
                theme.shell.secondary_text
            };
            Icon::new(SymbolName::Bell)
                .size(theme.layout.avatar_size)
                .color(foreground)
                .paint(
                    Rect::new(
                        panel.origin.x,
                        if self.is_tablet() {
                            center_y - theme.layout.avatar_size
                        } else {
                            panel.origin.y + panel.size.height * 0.38
                        },
                        panel.size.width,
                        theme.layout.avatar_size,
                    ),
                    context,
                );
            Text::styled("No Notifications", TextRole::Label)
                .alignment(TextAlignment::Center)
                .color(foreground)
                .paint(
                    Rect::new(
                        panel.origin.x,
                        if self.is_tablet() {
                            center_y + theme.spacing.medium
                        } else {
                            panel.origin.y + panel.size.height * 0.48
                        },
                        panel.size.width,
                        24.0,
                    ),
                    context,
                );
            return;
        }
        let body = self.body_rect(panel);
        context
            .display_list
            .push(viewkit::draw_command::DrawCommand::PushClip { rect: body });
        for (entry, row) in self.visible_entries(panel, &notifications) {
            if let CenterEntry::Notification(id) = entry {
                if let Some(notification) = notifications.iter().find(|item| item.id == id) {
                    self.paint_notification(
                        notification,
                        self.app(notification, &apps),
                        row,
                        context,
                    );
                }
            }
            let (_, control_bounds, control) = self.row_control(&entry, row, &apps);
            if control_bounds.intersection(body) == Some(control_bounds) {
                control.paint(control_bounds, context);
            }
        }
        context
            .display_list
            .push(viewkit::draw_command::DrawCommand::PopClip);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.open.get() {
            let now = Instant::now();
            let banner = *self.interaction.banner.borrow();
            let banner_hit = banner.map(|banner| {
                let (rect, _) = Self::banner_rect(bounds, banner, now);
                (banner.id, rect)
            });
            match event {
                ViewEvent::PointerMoved { position } => {
                    let hovered = banner_hit.is_some_and(|(_, rect)| {
                        rect.contains_rounded(
                            Theme::current().surface.floating_radius.resolve(
                                &Theme::current().radius,
                                rect.size.width,
                                rect.size.height,
                            ),
                            *position,
                        )
                    });
                    let changed = self.interaction.banner_hovered.get() != hovered;
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
                        .filter(|(_, rect)| {
                            rect.contains_rounded(
                                Theme::current().surface.floating_radius.resolve(
                                    &Theme::current().radius,
                                    rect.size.width,
                                    rect.size.height,
                                ),
                                *position,
                            )
                        })
                        .map(|(id, _)| id);
                    self.interaction.banner_pressed.set(pressed);
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
                        .filter(|(_, rect)| {
                            rect.contains_rounded(
                                Theme::current().surface.floating_radius.resolve(
                                    &Theme::current().radius,
                                    rect.size.width,
                                    rect.size.height,
                                ),
                                *position,
                            )
                        })
                        .map(|(id, _)| id);
                    let pressed = self.interaction.banner_pressed.replace(None);
                    if pressed.is_some() && pressed == released {
                        self.interaction.banner.replace(None);
                        self.interaction.banner_hovered.set(false);
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
                    if let Some(active) = self.interaction.banner.borrow_mut().as_mut()
                        && active.dismiss_started_at.is_none()
                    {
                        active.dismiss_started_at = Some(now);
                        active.paused_remaining = None;
                    }
                    self.interaction.banner_hovered.set(false);
                    context.request_redraw_in(Self::banner_damage(bounds));
                    return EventResult::Consumed;
                }
                _ => {}
            }
            return self.content.handle_event(bounds, event, context);
        }
        let notifications = self.platform.borrow().notifications();
        let (panel, _, _) = self.panel_presentation(bounds, Instant::now());
        let bar_position = match event {
            ViewEvent::PointerMoved { position }
            | ViewEvent::PointerPressed { position, .. }
            | ViewEvent::PointerReleased { position, .. }
            | ViewEvent::PointerFocusRequested { position } => Some(*position),
            _ => None,
        };
        if !self.is_tablet()
            && bar_position.is_some_and(|point| point.y < bounds.origin.y + PANEL_TOP)
        {
            return self.content.handle_event(bounds, event, context);
        }
        let apps = self.apps.get();
        let mut controls = Vec::new();
        if !notifications.is_empty() {
            controls.push((Hit::Clear, self.clear_rect(panel), self.clear_control()));
        }
        let body = self.body_rect(panel);
        for (entry, row) in self.visible_entries(panel, &notifications) {
            let control = self.row_control(&entry, row, &apps);
            if control.1.intersection(body) == Some(control.1) {
                controls.push(control);
            }
        }
        if let ViewEvent::PointerMoved { position } = event {
            let hit = self.hit(bounds, *position, &notifications);
            if self.hovered.replace(hit) != hit {
                context.request_redraw_in(self.panel_damage(bounds));
            }
        }
        let mut consumed = false;
        for (hit, frame, control) in controls {
            consumed |= control.handle_event(frame, event, context).is_consumed();
            if control.interaction().take_clicked() {
                self.apply(hit, &notifications);
                context.request_redraw_in(self.panel_damage(bounds));
                return EventResult::Consumed;
            }
        }
        if consumed {
            return EventResult::Consumed;
        }
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
                if !panel.contains_rounded(
                    Theme::current().surface.floating_radius.resolve(
                        &Theme::current().radius,
                        panel.size.width,
                        panel.size.height,
                    ),
                    *position,
                ) {
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
                    if let Some(hit) = released {
                        self.apply(hit, &notifications);
                    }
                }
                context.request_redraw();
            }
            ViewEvent::Scroll {
                position, delta_y, ..
            } if panel.contains_rounded(
                Theme::current().surface.floating_radius.resolve(
                    &Theme::current().radius,
                    panel.size.width,
                    panel.size.height,
                ),
                *position,
            ) =>
            {
                let maximum = self.maximum_scroll(panel, &self.entries(&notifications));
                let next = (self.interaction.scroll.get() - delta_y).clamp(0.0, maximum);
                if self.interaction.scroll.replace(next) != next {
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

    fn notice(id: u64, bundle: &str, created_at: u64) -> UserNotification {
        UserNotification {
            id,
            created_at,
            read: false,
            bundle_id: bundle.into(),
            title: format!("Notice {id}"),
            body: "Body".into(),
        }
    }

    #[test]
    fn groups_sort_by_latest_delivery_and_expand_in_chronological_order() {
        let notifications = vec![notice(3, "a", 30), notice(2, "b", 20), notice(1, "a", 10)];
        assert_eq!(
            grouped_entries(&notifications, &HashSet::new()),
            [
                CenterEntry::Group {
                    latest: 3,
                    bundle: "a".into(),
                    count: 2
                },
                CenterEntry::Notification(3),
                CenterEntry::Notification(2),
            ]
        );
        assert_eq!(
            grouped_entries(&notifications, &HashSet::from(["a".into()])),
            [
                CenterEntry::Group {
                    latest: 3,
                    bundle: "a".into(),
                    count: 2
                },
                CenterEntry::Notification(3),
                CenterEntry::Notification(1),
                CenterEntry::Notification(2),
            ]
        );
    }

    #[test]
    fn equal_timestamps_preserve_service_delivery_order_instead_of_process_id_order() {
        assert_eq!(
            grouped_entries(&[notice(1, "a", 20), notice(999, "b", 20)], &HashSet::new()),
            [CenterEntry::Notification(1), CenterEntry::Notification(999)]
        );
    }

    #[test]
    fn notification_panel_fits_small_displays() {
        for bounds in [
            Rect::new(0.0, 0.0, 320.0, 240.0),
            Rect::new(15.0, 20.0, 1440.0, 1024.0),
        ] {
            let panel = NotificationCenterLayer::<Rectangle>::panel(bounds);
            assert!(panel.origin.x >= bounds.origin.x);
            assert!(panel.origin.x + panel.size.width <= bounds.origin.x + bounds.size.width);
            assert!(panel.origin.y + panel.size.height <= bounds.origin.y + bounds.size.height);
        }
    }

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
