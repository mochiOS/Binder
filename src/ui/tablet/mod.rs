//! Tablet shell presentation. Startup selects this root once; application,
//! notification and window ownership remain in the existing desktop adapters.
mod gesture;
mod keyboard;
mod model;
mod render;

use crate::{
    control_center_preferences::ControlCenterPreferences,
    dock_preferences::DockPreferences,
    platform::{AppInfo, DesktopPlatform, ProcessId, RemoteWindowId, SystemBarState},
    window::{DesktopWindow, DesktopWindows, WindowId},
};
use gesture::{Action, Gestures, Origin};
use model::{HomeLayout, ItemKind, Slot};
use render::{Geometry, Hit};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};
use viewkit::components::EmptyView;
use viewkit::software_keyboard::KeyboardAction;
use viewkit::{
    animation::SpringAnimation,
    event::{EventContext, EventResult, ViewEvent},
    platform::{Key, PointerButton, TouchPhase},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const MOUSE: u64 = u64::MAX;
const NAVIGATION_REVEAL_TIMEOUT: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Surface {
    #[default]
    Home,
    Application,
    Library,
    Notifications,
    Switcher,
    Folder(u64),
    Controls,
}

impl Surface {
    fn sheet_travel(self, height: f32) -> f32 {
        let fraction = if self == Self::Controls { 0.65 } else { 1.0 };
        (height * fraction).max(1.0)
    }
}

#[derive(Default)]
struct Motion {
    value: f32,
    target: f32,
    velocity: f32,
    transition: Option<SpringAnimation>,
}
impl Motion {
    fn set(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.transition = None;
        self.velocity = 0.0;
    }
    fn follow(&mut self, value: f32, velocity: f32) {
        self.set(value);
        self.velocity = velocity;
    }
    fn animate(&mut self, target: f32, now: Instant) {
        self.sample(now);
        self.target = target;
        let motion = Theme::current().motion.standard;
        self.transition = Some(SpringAnimation::new(
            self.value,
            target,
            self.velocity,
            now,
            motion.duration,
        ));
    }
    fn animate_application(&mut self, target: f32, now: Instant) {
        self.sample(now);
        self.target = target;
        self.transition = Some(SpringAnimation::new(
            self.value,
            target,
            self.velocity,
            now,
            Theme::current().motion.slow.duration,
        ));
    }
    fn settle(&mut self, target: f32, now: Instant) {
        self.sample(now);
        self.target = target;
        self.velocity = 0.0;
        self.transition = Some(SpringAnimation::new(
            self.value,
            target,
            0.0,
            now,
            Theme::current().motion.standard.duration.mul_f32(0.35),
        ));
    }
    fn sample(&mut self, now: Instant) -> Option<Instant> {
        let animation = self.transition?;
        let sample = animation.sample(now);
        self.value = sample.value;
        self.velocity = sample.velocity;
        if sample.next_redraw_at.is_none() {
            self.transition = None;
        }
        sample.next_redraw_at
    }
}

struct NavigationDrag {
    app_progress: f32,
    app_center: Point,
    bar_progress: f32,
}

struct NavigationBar {
    reveal: Motion,
    hide_at: Option<Instant>,
}

impl Default for NavigationBar {
    fn default() -> Self {
        Self {
            reveal: Motion {
                value: 1.0,
                target: 1.0,
                ..Motion::default()
            },
            hide_at: None,
        }
    }
}

impl NavigationBar {
    fn show(&mut self, visible: bool, now: Instant) {
        self.hide_at = None;
        let target = if visible { 1.0 } else { 0.0 };
        if !Theme::current().motion.omochi.enabled {
            self.reveal.set(target);
        } else if self.reveal.target != target {
            self.reveal.animate(target, now);
        }
    }

    fn follow(&mut self, start: f32, delta: f32, edge_height: f32) {
        self.hide_at = None;
        self.reveal
            .set((start - delta / edge_height).clamp(0.0, 1.0));
    }

    fn finish_gesture(&mut self, now: Instant) {
        self.reveal.sample(now);
        if self.reveal.value > 0.0 || self.reveal.target > 0.0 {
            self.show(true, now);
            self.hide_at = Some(now + NAVIGATION_REVEAL_TIMEOUT);
        }
    }

    fn sample(&mut self, now: Instant, touching_edge: bool) -> Option<Instant> {
        if self.hide_at.is_some_and(|deadline| now >= deadline) && !touching_edge {
            self.show(false, now);
        }
        let deadline = self.hide_at.filter(|deadline| *deadline > now);
        self.reveal.sample(now).into_iter().chain(deadline).min()
    }

    fn amount(&self, sheet_amount: f32) -> f32 {
        let amount = self.reveal.value.clamp(0.0, 1.0);
        amount + (1.0 - amount) * sheet_amount
    }
}

pub(crate) struct TabletState {
    icons: Rc<super::app_icon::AppIcons>,
    layout: HomeLayout,
    surface: Surface,
    previous: Surface,
    sheet: Motion,
    scroll: Motion,
    switch_scroll: Motion,
    app_motion: Motion,
    presented: Option<WindowId>,
    app_anchor: Option<Point>,
    app_initial_progress: Option<f32>,
    app_x: Motion,
    app_y: Motion,
    navigation_start: Option<NavigationDrag>,
    navigation_bar: NavigationBar,
    switch_origin: Option<(ProcessId, Rect)>,
    switch_previews: HashMap<RemoteWindowId, Option<viewkit::image::ImageData>>,
    gestures: Gestures,
    legacy_touches: HashSet<u64>,
    pan_start: f32,
    sheet_start: f32,
    list_start: f32,
    app_scroll: f32,
    folder_scroll: f32,
    dock_scroll: f32,
    editing: bool,
    dragged: Option<(u64, Point)>,
    last_size: Size,
    last_focused: Option<WindowId>,
    catalogue: Vec<String>,
    search: TextFieldInteractionState,
    keyboard: keyboard::Keyboard,
    last_application_bounds: Option<Rect>,
    notification_interaction: Rc<super::notification_center::NotificationCenterInteraction>,
    notification_settings: ButtonInteractionState,
    // Existing overlay components request dismissal through these retained
    // adapters. Surface remains the authoritative exclusive presentation.
    notification_open: State<bool>,
    control_open: State<bool>,
    press: Option<(Rect, viewkit::omochi::OmochiSurface)>,
    error: Option<String>,
    keyboard_captured: Cell<bool>,
}
impl Default for TabletState {
    fn default() -> Self {
        Self {
            icons: Rc::new(super::app_icon::AppIcons::default()),
            layout: HomeLayout::load(),
            surface: Surface::Home,
            previous: Surface::Home,
            sheet: Motion::default(),
            scroll: Motion::default(),
            switch_scroll: Motion::default(),
            app_motion: Motion::default(),
            presented: None,
            app_anchor: None,
            app_initial_progress: None,
            app_x: Motion::default(),
            app_y: Motion::default(),
            navigation_start: None,
            navigation_bar: NavigationBar::default(),
            switch_origin: None,
            switch_previews: HashMap::new(),
            gestures: Gestures::default(),
            legacy_touches: HashSet::new(),
            pan_start: 0.,
            sheet_start: 0.,
            list_start: 0.,
            app_scroll: 0.,
            folder_scroll: 0.,
            dock_scroll: 0.,
            editing: false,
            dragged: None,
            last_size: Size::ZERO,
            last_focused: None,
            catalogue: Vec::new(),
            search: TextFieldInteractionState::new(),
            keyboard: keyboard::Keyboard::default(),
            last_application_bounds: None,
            notification_interaction: Rc::new(
                super::notification_center::NotificationCenterInteraction::default(),
            ),
            notification_settings: ButtonInteractionState::new(),
            notification_open: State::new(false),
            control_open: State::new(false),
            press: None,
            error: None,
            keyboard_captured: Cell::new(false),
        }
    }
}

impl TabletState {
    fn clear_gestures(&mut self, now: Instant) {
        if self.gestures.edge_active() && self.surface == Surface::Application {
            self.navigation_bar.finish_gesture(now);
        }
        self.gestures.clear();
        self.navigation_start = None;
    }
}

pub(crate) struct Resources {
    pub platform: Rc<RefCell<dyn DesktopPlatform>>,
    pub system_bar: State<SystemBarState>,
    pub windows: State<DesktopWindows>,
    pub apps: State<Vec<AppInfo>>,
    pub running: State<Vec<String>>,
    pub dock: State<DockPreferences>,
    pub pending: Rc<RefCell<super::app_library::PendingAppActivation>>,
    pub fast_poll: Rc<Cell<Option<Instant>>>,
    pub test_states: Rc<RefCell<HashMap<WindowId, super::test::TestWindowState>>>,
    pub failures: Rc<RefCell<HashMap<WindowId, super::launch_failure::LaunchFailureWindowState>>>,
    pub control_preferences: Rc<RefCell<ControlCenterPreferences>>,
    pub control_interaction: Rc<RefCell<super::control_center::ControlCenterInteraction>>,
    pub control_editor: Rc<RefCell<super::control_center_editor::ControlCenterEditorInteraction>>,
    pub wallpaper: super::wallpaper::Wallpaper,
}

pub(crate) struct TabletShell {
    resources: Resources,
    state: Rc<RefCell<TabletState>>,
    refresh: super::desktop::PlatformRefreshView,
    icons: Rc<super::app_icon::AppIcons>,
    notification_open: State<bool>,
    notifications: super::notification_center::NotificationCenterLayer<EmptyView>,
    control_open: State<bool>,
    controls: super::control_center::ControlCenterLayer<EmptyView>,
}

impl TabletShell {
    pub fn new(resources: Resources, state: Rc<RefCell<TabletState>>) -> Self {
        let notification_open = state.borrow().notification_open.clone();
        let control_open = state.borrow().control_open.clone();
        let notifications = super::notification_center::NotificationCenterLayer::new(
            EmptyView,
            notification_open.clone(),
            resources.platform.clone(),
            resources.apps.clone(),
            state.borrow().notification_interaction.clone(),
            resources.pending.clone(),
        )
        .with_tablet_presentation();
        let controls = super::control_center::ControlCenterLayer::new(
            EmptyView,
            control_open.clone(),
            notification_open.clone(),
            resources.system_bar.clone(),
            resources.control_preferences.clone(),
            resources.control_interaction.clone(),
            resources.platform.clone(),
            resources.windows.clone(),
            resources.apps.clone(),
            resources.fast_poll.clone(),
        )
        .with_external_transition();
        let refresh = super::desktop::PlatformRefreshView::new(
            resources.platform.clone(),
            resources.system_bar.clone(),
            resources.windows.clone(),
            State::new(None),
            State::new(None),
            resources.apps.clone(),
            resources.running.clone(),
            resources.fast_poll.clone(),
        );
        let icons = state.borrow().icons.clone();
        Self {
            resources,
            state,
            refresh,
            icons,
            notification_open,
            notifications,
            control_open,
            controls,
        }
    }

    fn dock_apps(&self) -> Vec<AppInfo> {
        super::dock::ordered_applications(
            &self.resources.apps.get(),
            &self.resources.dock.get(),
            &self.resources.running.get(),
        )
    }

    fn entries(&self) -> Vec<super::app_switcher::Entry> {
        super::app_switcher::running_entries(
            &self.resources.platform,
            &self.resources.windows.get(),
            &self.resources.apps.get(),
            &self.resources.running.get(),
        )
    }

    fn show(&self, surface: Surface, now: Instant) {
        let mut state = self.state.borrow_mut();
        if matches!(surface, Surface::Folder(_)) && surface != state.surface {
            state.folder_scroll = 0.0;
        }
        if matches!(
            surface,
            Surface::Library
                | Surface::Notifications
                | Surface::Switcher
                | Surface::Folder(_)
                | Surface::Controls
        ) {
            if matches!(state.surface, Surface::Home | Surface::Application) {
                state.previous = state.surface;
            }
            state.surface = surface;
            state.sheet.animate(1.0, now);
        } else {
            state.previous = surface;
            state.sheet.animate(0.0, now);
            // Keep the outgoing overlay through its closing animation.
            if matches!(state.surface, Surface::Home | Surface::Application) {
                state.surface = surface;
            }
        }
        self.notification_open
            .update_for_repaint(|open| *open = surface == Surface::Notifications);
        self.control_open
            .update_for_repaint(|open| *open = surface == Surface::Controls);
    }

    fn home(&self, now: Instant, geometry: &Geometry) {
        let destination = self.home_destination(geometry);
        self.resources.windows.update_for_repaint(|desktop| {
            for window in &mut desktop.windows {
                window.minimized = true;
            }
            desktop.focused = None;
        });
        self.flush_window_state();
        {
            let mut state = self.state.borrow_mut();
            state.editing = false;
            state.last_focused = None;
            state.navigation_start = None;
            state.navigation_bar.show(true, now);
            state.app_motion.animate_application(0.0, now);
            state.app_x.animate_application(destination.x, now);
            state.app_y.animate_application(destination.y, now);
        }
        self.show(Surface::Home, now);
    }

    fn flush_window_state(&self) {
        let (resized, focused) = self
            .resources
            .windows
            .update_for_repaint(|desktop| desktop.take_window_state_notifications());
        {
            let mut platform = self.resources.platform.borrow_mut();
            for notification in resized {
                if let Err(error) = platform.notify_window_resized(notification) {
                    self.state.borrow_mut().error =
                        Some(format!("Application resize failed: {error:?}"));
                }
            }
            for notification in focused {
                if let Err(error) = platform.notify_window_focus_changed(notification) {
                    self.state.borrow_mut().error =
                        Some(format!("Application focus failed: {error:?}"));
                }
            }
        }
    }

    fn activate(&self, app: &AppInfo, now: Instant) {
        self.resources.pending.borrow_mut().queue(app.clone());
        self.state.borrow_mut().editing = false;
        self.show(Surface::Application, now);
    }

    fn content(&self, window: &DesktopWindow) -> Box<dyn View> {
        super::window::content(
            window,
            self.resources.test_states.borrow().get(&window.id).cloned(),
            self.resources.failures.borrow().get(&window.id).cloned(),
            self.resources.windows.clone(),
            self.resources.control_preferences.clone(),
            self.resources.control_editor.clone(),
            self.resources.apps.clone(),
        )
    }

    fn foreground(&self) -> Option<DesktopWindow> {
        self.resources.windows.with(|desktop| {
            desktop
                .windows
                .iter()
                .rev()
                .find(|window| !window.minimized && Some(window.id) == desktop.focused)
                .cloned()
        })
    }

    fn prepare(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let now = Instant::now();
        let geometry = self.geometry(bounds, context.theme, context.environment());
        if let Err(error) = self
            .resources
            .platform
            .borrow_mut()
            .set_fullscreen_work_area(Some(geometry.application), context.scale_factor())
        {
            self.state.borrow_mut().error =
                Some(format!("Fullscreen presentation is unavailable: {error:?}"));
        }
        self.refresh.paint(bounds, context);
        let applications = self.resources.apps.get();
        let text_target = self.text_target();
        {
            let mut state = self.state.borrow_mut();
            state.keyboard.update_target(text_target);
            if state
                .catalogue
                .iter()
                .map(String::as_str)
                .ne(applications.iter().map(|app| app.bundle_id.as_str()))
            {
                state.keyboard.load(&applications);
            }
        }
        let updated = self.geometry(bounds, context.theme, context.environment());
        if updated.application != geometry.application {
            let _ = self
                .resources
                .platform
                .borrow_mut()
                .set_fullscreen_work_area(Some(updated.application), context.scale_factor());
        }
        let geometry = updated;
        let catalogue: Vec<_> = applications
            .iter()
            .map(|app| app.bundle_id.clone())
            .collect();
        let focused = self.resources.windows.with(|desktop| desktop.focused);
        {
            let mut state = self.state.borrow_mut();
            if state.catalogue != catalogue {
                if state
                    .layout
                    .synchronize(catalogue.iter().map(String::as_str))
                {
                    if let Err(error) = state.layout.save() {
                        state.error = Some(format!("Home layout could not be saved: {error}"));
                    }
                }
                state.catalogue = catalogue;
            }
            if state.last_size != geometry.bounds.size {
                let previous = Geometry::new(
                    Rect::new(0., 0., state.last_size.width, state.last_size.height),
                    context.theme,
                    self.dock_apps().len(),
                );
                let relative = state.scroll.value / previous.canvas.size.width.max(1.0);
                let columns = state
                    .layout
                    .items
                    .iter()
                    .map(|item| item.slot.column + 1)
                    .max()
                    .unwrap_or(0);
                let maximum =
                    model::maximum_scroll(geometry.canvas.size.width, geometry.pitch, columns);
                state.scroll.set(model::snap(
                    relative * geometry.canvas.size.width,
                    0.0,
                    geometry.canvas.size.width,
                    maximum,
                ));
                state.last_size = geometry.bounds.size;
                state.clear_gestures(now);
                state.dragged = None;
            }
            if state.last_application_bounds != Some(geometry.application) {
                state.last_application_bounds = Some(geometry.application);
                if state.app_motion.target == 1.0 && state.navigation_start.is_none() {
                    let center = Point::new(
                        geometry.application.origin.x + geometry.application.size.width * 0.5,
                        geometry.application.origin.y + geometry.application.size.height * 0.5,
                    );
                    // Opening a keyboard can resize the app during its launch.
                    // Move the animation destination with the fullscreen area.
                    if state.app_motion.transition.is_some() {
                        state.app_x.animate_application(center.x, now);
                        state.app_y.animate_application(center.y, now);
                    } else {
                        state.app_x.set(center.x);
                        state.app_y.set(center.y);
                    }
                }
            }
            if focused.is_some() && focused != state.last_focused {
                let returning = state.presented == focused
                    && state.app_motion.target == 0.0
                    && state.app_motion.value > 0.0;
                state.presented = focused;
                let progress = state.app_initial_progress.take().unwrap_or(0.0);
                if !returning {
                    state.app_motion.set(progress);
                }
                state.app_motion.animate_application(1.0, now);
                state.navigation_bar.show(false, now);
                let center = Point::new(
                    geometry.application.origin.x + geometry.application.size.width * 0.5,
                    geometry.application.origin.y + geometry.application.size.height * 0.5,
                );
                let anchor = state.app_anchor.take().unwrap_or(center);
                if !returning {
                    state.app_x.set(anchor.x);
                    state.app_y.set(anchor.y);
                }
                state.app_x.animate_application(center.x, now);
                state.app_y.animate_application(center.y, now);
                state.switch_previews.clear();
                state.surface = Surface::Application;
                state.previous = Surface::Application;
                state.sheet.set(0.0);
                self.notification_open
                    .update_for_repaint(|open| *open = false);
                self.control_open.update_for_repaint(|open| *open = false);
            }
            state.last_focused = focused;
            if state.sheet.target > 0.0
                && ((state.surface == Surface::Notifications && !self.notification_open.get())
                    || (state.surface == Surface::Controls && !self.control_open.get()))
            {
                state.sheet.animate(0.0, now);
            }
            let touching_edge = state.gestures.edge_active();
            if let Some(deadline) = state
                .scroll
                .sample(now)
                .into_iter()
                .chain(state.sheet.sample(now))
                .chain(state.switch_scroll.sample(now))
                .chain(state.app_motion.sample(now))
                .chain(state.app_x.sample(now))
                .chain(state.app_y.sample(now))
                .chain(state.navigation_bar.sample(now, touching_edge))
                .min()
            {
                context.request_redraw_at(deadline);
            }
            if state.sheet.value == 0.0
                && state.sheet.transition.is_none()
                && !state.gestures.active()
            {
                if state.surface == Surface::Switcher {
                    state.switch_previews.clear();
                }
                state.surface = state.previous;
            }
            self.notification_open
                .update_for_repaint(|open| *open = state.surface == Surface::Notifications);
            self.control_open
                .update_for_repaint(|open| *open = state.surface == Surface::Controls);
            if let Some(deadline) = state.gestures.deadline().filter(|deadline| *deadline > now) {
                context.request_redraw_at(deadline);
            }
        }
        let action = self.state.borrow_mut().gestures.tick(now);
        if let Some(action) = action {
            self.apply(action, &geometry, now);
            context.request_redraw_at(now);
        }
        let step = self.resources.pending.borrow_mut().advance();
        match step {
            super::app_library::PendingActivationStep::Idle => {}
            super::app_library::PendingActivationStep::PresentClosedFrame => {
                context.request_redraw_at(now)
            }
            super::app_library::PendingActivationStep::Launch(app) => {
                super::app_library::perform_activation(
                    &app,
                    &self.resources.platform,
                    &self.resources.windows,
                    &self.resources.fast_poll,
                    &self.resources.failures,
                );
                context.request_redraw_at(now);
            }
        }
        // Reuse the normal resize/focus notification path. No app restart is
        // involved, and non-resizable desktop requests still fill tablet space.
        self.resources.windows.update_for_repaint(|desktop| {
            for window in &mut desktop.windows {
                window.frame = geometry.application;
            }
        });
        self.flush_window_state();
        if self.foreground().is_none()
            && focused.is_none()
            && self.state.borrow().surface == Surface::Application
        {
            self.home(now, &geometry);
        }
        let application = self.state.borrow().surface == Surface::Application;
        let deferred_region = (application
            && self
                .foreground()
                .is_some_and(|window| window.externally_composited()))
        .then_some(geometry.edge);
        let _ = self
            .resources
            .platform
            .borrow_mut()
            .set_deferred_window_input(deferred_region, gesture::SLOP, context.scale_factor());
        let app_maximum = self.library_maximum(&geometry);
        let folder_maximum = self.folder_maximum(&geometry);
        let switch_maximum = self.entries().len().saturating_sub(1) as f32 * geometry.card_pitch();
        let mut state = self.state.borrow_mut();
        state.app_scroll = state.app_scroll.clamp(0.0, app_maximum);
        state.folder_scroll = state.folder_scroll.clamp(0.0, folder_maximum);
        if state.switch_scroll.value > switch_maximum {
            state.switch_scroll.set(switch_maximum);
        }
    }

    fn maximum_scroll(&self, geometry: &Geometry) -> f32 {
        let columns = self
            .state
            .borrow()
            .layout
            .items
            .iter()
            .map(|item| item.slot.column + 1)
            .max()
            .unwrap_or(0);
        model::maximum_scroll(geometry.canvas.size.width, geometry.pitch, columns)
    }

    fn apply(&self, action: Action, geometry: &Geometry, now: Instant) {
        match action {
            Action::FinishNavigation {
                delta,
                velocity,
                delta_x,
                velocity_x,
                home,
            } => {
                // The last input sample must reach presentation even if no
                // paint occurred between the last move and the release.
                self.apply(
                    Action::Navigate {
                        delta,
                        velocity,
                        delta_x,
                        velocity_x,
                    },
                    geometry,
                    now,
                );
                self.state.borrow_mut().navigation_start = None;
                if home {
                    self.home(now, geometry);
                } else {
                    self.apply(Action::Cancel, geometry, now);
                }
            }
            Action::Switcher => {
                let origin = self.foreground().and_then(|window| {
                    window
                        .process_id
                        .map(|pid| (pid, self.app_transform(geometry).rect(geometry.application)))
                });
                {
                    let mut state = self.state.borrow_mut();
                    state.switch_origin = origin;
                    state.switch_scroll.set(0.0);
                    state.switch_previews.clear();
                }
                self.home(now, geometry);
                self.show(Surface::Switcher, now);
            }
            Action::Navigate {
                delta,
                velocity,
                delta_x,
                velocity_x,
            } => {
                if self.state.borrow().surface == Surface::Application {
                    let mut state = self.state.borrow_mut();
                    let Some(&NavigationDrag {
                        app_progress: start_progress,
                        app_center: center,
                        bar_progress,
                    }) = state.navigation_start.as_ref() else {
                        return;
                    };
                    state.navigation_bar.follow(bar_progress, delta, geometry.edge.size.height);
                    let minimum = start_progress.min(0.35);
                    let progress =
                        (start_progress + delta / geometry.bounds.size.height).clamp(minimum, 1.0);
                    state
                        .app_x
                        .follow(center.x + delta_x * 0.35, velocity_x * 0.35);
                    state.app_y.follow(center.y + delta * 0.35, velocity * 0.35);
                    state.app_motion.follow(
                        progress,
                        if progress > minimum && progress < 1.0 {
                            velocity / geometry.bounds.size.height
                        } else {
                            0.0
                        },
                    );
                }
            }
            Action::RevealNavigation => {
                let mut state = self.state.borrow_mut();
                if state.surface == Surface::Application {
                    state.navigation_bar.show(true, now);
                }
            }
            Action::Tap(point) => self.tap(point, geometry, now),
            Action::EdgeTap(_) => {}
            Action::QuickSwitch(direction) => {
                let entries = self.entries();
                if !entries.is_empty() {
                    let focused = self
                        .resources
                        .windows
                        .with(DesktopWindows::focused_process_id);
                    let current = entries
                        .iter()
                        .position(|entry| Some(entry.process) == focused)
                        .unwrap_or(0);
                    let next =
                        (current as i32 + direction).rem_euclid(entries.len() as i32) as usize;
                    self.activate(&entries[next].app, now);
                }
            }
            Action::Pan {
                delta,
                velocity,
                finished,
            } => {
                if self.state.borrow().surface == Surface::Switcher {
                    let count = self.entries().len();
                    let mut state = self.state.borrow_mut();
                    let maximum = (count.saturating_sub(1) as f32 * geometry.card_pitch()).max(0.0);
                    let position = (state.pan_start + delta).clamp(0.0, maximum);
                    if finished {
                        state.switch_scroll.velocity = velocity;
                        state.switch_scroll.settle(
                            (position / geometry.card_pitch()).round() * geometry.card_pitch(),
                            now,
                        );
                    } else {
                        state.switch_scroll.follow(position, velocity);
                    }
                } else {
                    let maximum = self.maximum_scroll(geometry);
                    let mut state = self.state.borrow_mut();
                    let position = state.pan_start + delta;
                    if finished {
                        state.scroll.velocity = velocity;
                        state.scroll.settle(
                            model::snap(position, velocity, geometry.canvas.size.width, maximum),
                            now,
                        );
                    } else {
                        state.scroll.follow(
                            model::resisted(position, maximum, geometry.canvas.size.width),
                            velocity,
                        );
                    }
                }
            }
            Action::Sheet {
                origin,
                delta,
                velocity,
                finished,
            } => {
                let (origin, delta, velocity) = if origin == Origin::NotificationList {
                    let start = self.state.borrow().list_start;
                    let offset = self
                        .notifications
                        .set_scroll_offset(self.notification_bounds(geometry), start - delta);
                    let remaining = (delta + offset - start).min(0.0);
                    let velocity = if remaining < 0.0 { velocity } else { 0.0 };
                    (Origin::Notifications, remaining, velocity)
                } else {
                    (origin, delta, velocity)
                };
                if origin == Origin::Dock {
                    let maximum = (self.dock_apps().len() as f32 * geometry.dock_pitch
                        - geometry.dock.size.height
                        + geometry.gap * 2.0)
                        .max(0.0);
                    let mut state = self.state.borrow_mut();
                    state.dock_scroll = (state.list_start - delta).clamp(0.0, maximum);
                    return;
                }
                if origin == Origin::Folder {
                    let maximum = self.folder_maximum(geometry);
                    let mut state = self.state.borrow_mut();
                    state.folder_scroll = (state.list_start - delta).clamp(0.0, maximum);
                    return;
                }
                if origin == Origin::LibraryList {
                    let maximum = self.library_maximum(geometry);
                    let mut state = self.state.borrow_mut();
                    state.app_scroll = (state.list_start - delta).clamp(0.0, maximum);
                    return;
                }
                let (surface, opening) = match origin {
                    Origin::HomeLibrary => (Surface::Library, true),
                    Origin::HomeNotifications => (Surface::Notifications, true),
                    Origin::TopNotifications => (Surface::Notifications, true),
                    Origin::TopControls => (Surface::Controls, true),
                    Origin::Library => (Surface::Library, false),
                    Origin::Notifications => (Surface::Notifications, false),
                    _ => return,
                };
                let sign = if surface == Surface::Library {
                    -1.0
                } else {
                    1.0
                };
                if opening && self.state.borrow().surface != surface {
                    let mut state = self.state.borrow_mut();
                    state.previous = if self.foreground().is_some() {
                        Surface::Application
                    } else {
                        Surface::Home
                    };
                    state.surface = surface;
                    self.notification_open
                        .update_for_repaint(|open| *open = surface == Surface::Notifications);
                    self.control_open
                        .update_for_repaint(|open| *open = surface == Surface::Controls);
                }
                let mut state = self.state.borrow_mut();
                let travel = surface.sheet_travel(geometry.bounds.size.height);
                let amount = (if opening { 0.0 } else { state.sheet_start }
                    + delta * sign / travel)
                .clamp(0.0, 1.0);
                let speed = velocity * sign / travel;
                state.sheet.follow(amount, speed);
                if finished {
                    let threshold = if opening { 0.24 } else { 0.80 };
                    let open = amount + speed * 0.12 >= threshold;
                    state.sheet.animate(if open { 1.0 } else { 0.0 }, now);
                    if !open {
                        self.notification_open
                            .update_for_repaint(|open| *open = false);
                    }
                }
            }
            Action::Edit(id, point) => {
                let mut state = self.state.borrow_mut();
                state.editing = true;
                state.dragged = Some((id, point));
            }
            Action::Drag(id, point) => {
                let maximum = self.maximum_scroll(geometry);
                let mut state = self.state.borrow_mut();
                state.dragged = Some((id, point));
                // Edge following while dragging exposes empty canvas slots.
                let shift = if point.x < geometry.canvas.origin.x + geometry.gap {
                    -geometry.pitch * 0.15
                } else if point.x
                    > geometry.canvas.origin.x + geometry.canvas.size.width - geometry.gap
                {
                    geometry.pitch * 0.15
                } else {
                    0.0
                };
                let next = (state.scroll.value + shift)
                    .clamp(0.0, maximum + geometry.canvas.size.width * 0.5);
                state.scroll.set(next);
            }
            Action::Drop(id, point) => {
                let mut state = self.state.borrow_mut();
                if geometry.dock.contains(point) {
                    let bundle = state.layout.items.iter().find_map(|item| {
                        if item.id == id {
                            if let ItemKind::Application(bundle) = &item.kind {
                                Some(bundle.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    });
                    if let Some(bundle) = bundle {
                        self.resources.dock.update_for_repaint(|preferences| {
                            if preferences.pin(&bundle) {
                                if let Err(error) = preferences.save() {
                                    state.error = Some(format!("Dock could not be saved: {error}"));
                                }
                            }
                        });
                    }
                }
                if geometry.canvas.contains(point) {
                    let slot = geometry.slot(point, state.scroll.value);
                    let target = geometry.home_cell(slot, state.scroll.value);
                    let combine = point.x > target.origin.x + target.size.width * 0.25
                        && point.x < target.origin.x + target.size.width * 0.75;
                    if state.layout.move_item(id, slot, combine) {
                        if let Err(error) = state.layout.save() {
                            state.error = Some(format!("Home layout could not be saved: {error}"));
                        }
                    }
                }
                state.dragged = None;
            }
            Action::Cancel => {
                let maximum = self.maximum_scroll(geometry);
                let mut state = self.state.borrow_mut();
                state.dragged = None;
                state.navigation_start = None;
                let target =
                    model::snap(state.scroll.value, 0.0, geometry.canvas.size.width, maximum);
                state.scroll.settle(target, now);
                if state.sheet.transition.is_none()
                    && state.sheet.value > 0.0
                    && state.sheet.value < 1.0
                {
                    let target = if state.sheet.value >= 0.5 { 1.0 } else { 0.0 };
                    state.sheet.animate(target, now);
                }
                if state.surface == Surface::Application {
                    state.app_motion.animate_application(1.0, now);
                    state.app_x.animate_application(
                        geometry.application.origin.x + geometry.application.size.width * 0.5,
                        now,
                    );
                    state.app_y.animate_application(
                        geometry.application.origin.y + geometry.application.size.height * 0.5,
                        now,
                    );
                }
            }
        }
    }

    fn tap(&self, point: Point, geometry: &Geometry, now: Instant) {
        let hit = self.hit(point, geometry);
        match hit {
            Some(Hit::Application(bundle)) => {
                if self.state.borrow().editing {
                    return;
                }
                if let Some(app) = self
                    .resources
                    .apps
                    .get()
                    .iter()
                    .find(|app| app.bundle_id == bundle)
                    .cloned()
                {
                    let anchor = self.launch_anchor(&bundle, point, geometry);
                    self.state.borrow_mut().app_anchor = Some(anchor);
                    if self.state.borrow().surface == Surface::Switcher {
                        if let Some(index) = self
                            .entries()
                            .iter()
                            .position(|entry| entry.app.bundle_id == bundle)
                        {
                            let mut state = self.state.borrow_mut();
                            let card = geometry.card(index, state.switch_scroll.value);
                            state.app_anchor = Some(Point::new(
                                card.origin.x + card.size.width * 0.5,
                                card.origin.y + card.size.height * 0.5,
                            ));
                            state.app_initial_progress = Some(
                                ((card.size.width / geometry.application.size.width
                                    - geometry.application_icon_scale())
                                    / (1.0 - geometry.application_icon_scale()).max(0.01))
                                .clamp(0.0, 1.0),
                            );
                        }
                    }
                    self.activate(&app, now);
                }
            }
            Some(Hit::Folder(id)) => {
                if !self.state.borrow().editing {
                    self.show(Surface::Folder(id), now);
                }
            }
            Some(Hit::Library) => self.show(Surface::Library, now),
            Some(Hit::Notifications) => self.show(Surface::Notifications, now),
            Some(Hit::Controls) => self.show(Surface::Controls, now),
            Some(Hit::Done) => {
                self.state.borrow_mut().editing = false;
            }
            Some(Hit::Close(process)) => {
                self.resources.windows.update(|desktop| {
                    let ids = desktop
                        .windows
                        .iter()
                        .filter(|window| window.process_id == Some(process))
                        .map(|window| window.id)
                        .collect::<Vec<_>>();
                    for id in ids {
                        desktop.close(id);
                    }
                });
            }
            Some(Hit::Search) => {}
            None if !matches!(
                self.state.borrow().surface,
                Surface::Home | Surface::Application
            ) =>
            {
                let previous = self.state.borrow().previous;
                self.show(previous, now);
            }
            None => {}
        }
    }

    fn search(&self) -> TextField {
        TextField::with_interaction(self.state.borrow().search.clone())
            .placeholder("Search applications")
            .leading_symbol(SymbolName::Search)
            .size(TextFieldSize::Touch)
    }

    fn geometry(&self, bounds: Rect, theme: &Theme, environment: WindowEnvironment) -> Geometry {
        let geometry =
            Geometry::for_environment(bounds, theme, self.dock_apps().len(), environment);
        let keyboard = self.state.borrow().keyboard.geometry(
            geometry.edge,
            geometry.bounds.size.height,
            theme,
        );
        geometry.with_keyboard(keyboard)
    }

    fn text_target(&self) -> Option<keyboard::Target> {
        let state = self.state.borrow();
        if state.surface == Surface::Library && state.search.is_focused() {
            return Some(keyboard::Target::Search);
        }
        if state.surface != Surface::Application {
            return None;
        }
        drop(state);
        let window = self.foreground()?;
        let pid = window.process_id?;
        let id = window.remote_window?;
        self.resources
            .platform
            .borrow()
            .accepts_software_keys(pid, id)
            .then_some(keyboard::Target::Application(pid, id))
    }

    fn keyboard_event(
        &self,
        geometry: &Geometry,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> bool {
        let contact = match event {
            ViewEvent::Touch { id, .. } => *id,
            _ => MOUSE,
        };
        if self.state.borrow().gestures.contains(contact) {
            return false;
        }
        let Some(presentation) = geometry.keyboard else {
            if matches!(
                event,
                ViewEvent::PointerPressed { .. }
                    | ViewEvent::Touch {
                        phase: TouchPhase::Started,
                        ..
                    }
            ) {
                self.state.borrow_mut().keyboard.resume();
            }
            return false;
        };
        let (consumed, action, kana, target) = {
            let mut state = self.state.borrow_mut();
            let keyboard = &mut state.keyboard;
            let selected = keyboard.panel.selected;
            let (consumed, action) =
                keyboard
                    .panel
                    .handle(&keyboard.layouts, presentation.keys, event, context.theme());
            if selected != keyboard.panel.selected {
                keyboard.save();
            }
            let kana = keyboard
                .layouts
                .get(keyboard.panel.selected)
                .is_some_and(|layout| layout.kana);
            (consumed, action, kana, keyboard.target)
        };
        if !consumed {
            if matches!(
                event,
                ViewEvent::PointerPressed { .. }
                    | ViewEvent::Touch {
                        phase: TouchPhase::Started,
                        ..
                    }
            ) {
                self.state.borrow_mut().keyboard.resume();
            }
            return false;
        }
        match action {
            Some(KeyboardAction::Dismiss) => self.state.borrow_mut().keyboard.dismiss(),
            Some(KeyboardAction::ToggleIme) => self.toggle_keyboard_ime(),
            Some(KeyboardAction::Key(key)) if target.is_some() && target == self.text_target() => {
                if kana
                    && ('ぁ'..='ゖ').contains(&key)
                    && viewkit::platform::input_method::enabled() == Some(false)
                {
                    self.toggle_keyboard_ime();
                }
                match target {
                    Some(keyboard::Target::Search) => {
                        let event = match key {
                            '\u{8}' => ViewEvent::Backspace,
                            '\n' => ViewEvent::KeyPressed {
                                key: Key::Enter,
                                modifiers: Default::default(),
                            },
                            _ => ViewEvent::TextInput {
                                text: key.to_string(),
                            },
                        };
                        self.search().handle_event(geometry.search, &event, context);
                    }
                    Some(keyboard::Target::Application(pid, id)) => {
                        self.resources
                            .platform
                            .borrow_mut()
                            .software_key(pid, id, key);
                    }
                    None => {}
                }
            }
            _ => {}
        }
        context.request_redraw();
        true
    }

    fn toggle_keyboard_ime(&self) {
        if let Some(japanese) = viewkit::platform::input_method::toggle() {
            self.resources.system_bar.update_for_repaint(|system| system.japanese_input = japanese);
        }
    }

    fn contact(
        &self,
        id: u64,
        phase: TouchPhase,
        position: Point,
        geometry: &Geometry,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let now = Instant::now();
        if phase == TouchPhase::Started {
            let surface = self.state.borrow().surface;
            let origin = if geometry.edge.contains(position) {
                Origin::Edge
            } else if matches!(surface, Surface::Home | Surface::Application)
                && position.y
                    < geometry.application.origin.y
                        + if surface == Surface::Home {
                            geometry.gap * 2.0
                        } else {
                            0.0
                        }
            {
                if position.x > geometry.bounds.origin.x + geometry.bounds.size.width * 0.75 {
                    Origin::TopControls
                } else {
                    Origin::TopNotifications
                }
            } else if matches!(surface, Surface::Application) {
                return self.foreground_event(
                    geometry,
                    &if id == MOUSE {
                        ViewEvent::PointerPressed {
                            position,
                            button: PointerButton::Primary,
                        }
                    } else {
                        ViewEvent::Touch {
                            id,
                            phase,
                            position,
                        }
                    },
                    context,
                );
            } else if surface == Surface::Library && geometry.search.contains(position) {
                return self.search().handle_event(
                    geometry.search,
                    &ViewEvent::Touch {
                        id,
                        phase,
                        position,
                    },
                    context,
                );
            } else if geometry.dock.contains(position) && surface == Surface::Home {
                Origin::Dock
            } else {
                match surface {
                    Surface::Home => Origin::Home(self.home_item_at(position, geometry)),
                    Surface::Library if self.state.borrow().app_scroll > 0.0 => Origin::LibraryList,
                    Surface::Library => Origin::Library,
                    Surface::Folder(_) => Origin::Folder,
                    Surface::Notifications
                        if self.notifications.scroll_bounds(self.notification_bounds(geometry)).contains(position)
                            && !self.resources.platform.borrow().notifications().is_empty() =>
                    {
                        Origin::NotificationList
                    }
                    Surface::Notifications => Origin::Notifications,
                    Surface::Switcher => Origin::Switcher,
                    _ => Origin::Tap,
                }
            };
            let mut state = self.state.borrow_mut();
            let editing = state.editing;
            let primary = state.gestures.begin(id, origin, position, editing, now);
            if primary {
                state.navigation_start =
                    if origin == Origin::Edge && surface == Surface::Application {
                        state.app_motion.sample(now);
                        state.app_x.sample(now);
                        state.app_y.sample(now);
                        state.navigation_bar.sample(now, true);
                        Some(NavigationDrag {
                            app_progress: state.app_motion.value,
                            app_center: Point::new(state.app_x.value, state.app_y.value),
                            bar_progress: state.navigation_bar.amount(0.0),
                        })
                    } else {
                        None
                    };
                state.pan_start = if surface == Surface::Switcher {
                    state.switch_scroll.value
                } else {
                    state.scroll.value
                };
                state.sheet_start = state.sheet.value;
                state.list_start = match origin {
                    Origin::Dock => state.dock_scroll,
                    Origin::Folder => state.folder_scroll,
                    Origin::NotificationList => self.notifications.scroll_offset(),
                    _ => state.app_scroll,
                };
            }
            if primary
                && matches!(origin, Origin::Home(Some(_)) | Origin::Dock)
                && context.theme().motion.omochi.enabled
            {
                let press = viewkit::omochi::OmochiSurface::new(
                    viewkit::omochi::DEFAULT_MATERIAL,
                    viewkit::omochi::DEFAULT_BURGERS,
                    viewkit::omochi::PullMode::Displacement,
                );
                press.begin_press(position, now);
                let bounds = if let Origin::Home(Some(id)) = origin {
                    state
                        .layout
                        .items
                        .iter()
                        .find(|item| item.id == id)
                        .map(|item| {
                            geometry.icon_rect(geometry.home_cell(item.slot, state.scroll.value))
                        })
                } else {
                    let index = ((position.y - geometry.dock.origin.y - geometry.gap * 0.5
                        + state.dock_scroll)
                        / geometry.dock_pitch)
                        .floor();
                    let size = geometry.dock.size.width - geometry.gap;
                    Some(Rect::new(
                        geometry.dock.origin.x + geometry.gap * 0.5,
                        geometry.dock.origin.y + geometry.gap * 0.5 + index * geometry.dock_pitch
                            - state.dock_scroll,
                        size,
                        size,
                    ))
                };
                state.press = bounds.map(|bounds| (bounds, press));
            }
        } else if !self.state.borrow().gestures.contains(id) {
            if self.state.borrow().surface == Surface::Library {
                return self.search().handle_event(
                    geometry.search,
                    &ViewEvent::Touch {
                        id,
                        phase,
                        position,
                    },
                    context,
                );
            }
            return self.foreground_event(
                geometry,
                &ViewEvent::Touch {
                    id,
                    phase,
                    position,
                },
                context,
            );
        }
        let (action, ends_navigation) = {
            let mut state = self.state.borrow_mut();
            let mut ends_navigation = false;
            if state.gestures.owns(id) {
                let ending = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
                ends_navigation = ending && state.gestures.edge_active();
                if let Some((_, press)) = &state.press {
                    if ending {
                        press.end_press(now);
                    } else {
                        press.move_press(position, now);
                    }
                }
            }
            let action = match phase {
                TouchPhase::Started => None,
                TouchPhase::Moved => state.gestures.update(id, position, now),
                TouchPhase::Ended => state.gestures.end(id, position, now),
                TouchPhase::Cancelled => state.gestures.cancel(id),
            };
            (action, ends_navigation)
        };
        if let Some(Action::EdgeTap(point)) = action {
            if self.state.borrow().surface == Surface::Application {
                for event in [
                    ViewEvent::PointerMoved { position: point },
                    ViewEvent::PointerPressed {
                        position: point,
                        button: PointerButton::Primary,
                    },
                    ViewEvent::PointerReleased {
                        position: point,
                        button: PointerButton::Primary,
                    },
                ] {
                    self.foreground_event(geometry, &event, context);
                }
            }
            context.request_redraw();
            return EventResult::Consumed;
        }
        if let Some(Action::Tap(point)) = action {
            let surface = self.state.borrow().surface;
            if !matches!(surface, Surface::Home | Surface::Application)
                && self.state.borrow().sheet.value < 0.99
            {
                context.request_redraw();
                return EventResult::Consumed;
            }
            if (surface == Surface::Notifications || surface == Surface::Controls)
                && !matches!(self.hit(point, geometry), Some(Hit::Controls))
            {
                for event in [
                    ViewEvent::PointerMoved { position: point },
                    ViewEvent::PointerPressed {
                        position: point,
                        button: PointerButton::Primary,
                    },
                    ViewEvent::PointerReleased {
                        position: point,
                        button: PointerButton::Primary,
                    },
                ] {
                    if surface == Surface::Notifications {
                        self.notifications.handle_event(
                            self.notification_bounds(geometry),
                            &event,
                            context,
                        );
                    } else {
                        self.controls.handle_event(geometry.bounds, &event, context);
                    }
                }
                context.request_redraw();
                return EventResult::Consumed;
            }
        }
        if let Some(action) = action {
            self.apply(action, geometry, now);
        }
        if ends_navigation {
            let mut state = self.state.borrow_mut();
            if state.surface == Surface::Application {
                state.navigation_bar.finish_gesture(now);
            }
        }
        context.request_redraw();
        EventResult::Consumed
    }

    fn foreground_event(
        &self,
        geometry: &Geometry,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(window) = self.foreground() {
            self.content(&window)
                .handle_event(geometry.application, event, context)
        } else {
            EventResult::Ignored
        }
    }
}

impl View for TabletShell {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.maximum
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.prepare(bounds, context);
        self.paint_scene(
            self.geometry(bounds, context.theme, context.environment()),
            context,
        );
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let result = self.handle_shell_event(bounds, event, context);
        let state = self.state.borrow();
        let capture = state.sheet.target > 0.0
            && !matches!(state.surface, Surface::Home | Surface::Application);
        if state.keyboard_captured.replace(capture) != capture {
            context.set_keyboard_capture(capture);
        }
        result
    }
}

impl TabletShell {
    fn handle_shell_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let geometry = self.geometry(bounds, context.theme(), context.environment());
        if self.keyboard_event(&geometry, event, context) {
            return EventResult::Consumed;
        }
        if self.state.borrow().surface == Surface::Notifications
            && matches!(
                event,
                ViewEvent::KeyPressed { .. }
                    | ViewEvent::KeyReleased { .. }
                    | ViewEvent::KeyboardFocusRequested { .. }
                    | ViewEvent::FocusVisibilityChanged { .. }
                    | ViewEvent::FocusChanged { .. }
                    | ViewEvent::ReconcileComponents { .. }
            )
        {
            let button = self.notification_settings_button();
            let result =
                button.handle_event(self.notification_settings_bounds(&geometry), event, context);
            if button.interaction().take_clicked() {
                self.show(Surface::Controls, Instant::now());
                context.request_redraw();
                return EventResult::Consumed;
            }
            if result.is_consumed() {
                return result;
            }
        }
        if matches!(event, ViewEvent::FocusChanged { focused: false }) {
            let mut state = self.state.borrow_mut();
            state.clear_gestures(Instant::now());
            state.legacy_touches.clear();
            state.dragged = None;
            state.press = None;
            let surface = state.surface;
            drop(state);
            return match surface {
                Surface::Library => self.search().handle_event(geometry.search, event, context),
                Surface::Notifications => self.notifications.handle_event(
                    self.notification_bounds(&geometry),
                    event,
                    context,
                ),
                Surface::Controls => self.controls.handle_event(bounds, event, context),
                _ => self.foreground_event(&geometry, event, context),
            };
        }
        if let ViewEvent::Touch {
            id,
            phase,
            position,
        } = event
        {
            let banner_start = *phase == TouchPhase::Started
                && !self.notification_open.get()
                && self.notifications.banner_contains(bounds, *position);
            let mut state = self.state.borrow_mut();
            if banner_start && state.legacy_touches.len() < 16 {
                state.legacy_touches.insert(*id);
            }
            if state.legacy_touches.contains(id) {
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    state.legacy_touches.remove(id);
                }
                return EventResult::Ignored;
            }
            drop(state);
            if self.state.borrow().surface == Surface::Controls
                && !self.state.borrow().gestures.contains(*id)
                && (*phase != TouchPhase::Started || !geometry.edge.contains(*position))
            {
                return if self.state.borrow().sheet.value < 0.99 {
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                };
            }
            return self.contact(*id, *phase, *position, &geometry, context);
        }
        if !self.notification_open.get()
            && !self.state.borrow().gestures.contains(MOUSE)
            && matches!(
                event,
                ViewEvent::PointerMoved { .. }
                    | ViewEvent::PointerPressed { .. }
                    | ViewEvent::PointerReleased { .. }
                    | ViewEvent::PointerLeft
            )
        {
            let result = self.notifications.handle_event(bounds, event, context);
            if result == EventResult::Consumed {
                return result;
            }
        }
        if self.state.borrow().surface == Surface::Controls
            && !self.state.borrow().gestures.contains(MOUSE)
            && !matches!(event,ViewEvent::PointerPressed{position,..} if geometry.edge.contains(*position))
            && !matches!(
                event,
                ViewEvent::KeyPressed {
                    key: Key::Escape,
                    ..
                }
            )
        {
            return if self.state.borrow().sheet.value < 0.99 {
                EventResult::Consumed
            } else {
                self.controls.handle_event(bounds, event, context)
            };
        }
        if self.state.borrow().surface == Surface::Library
            && (event.position().is_none()
                || event
                    .position()
                    .is_some_and(|point| geometry.search.contains(point)))
        {
            let result = self.search().handle_event(geometry.search, event, context);
            if result == EventResult::Consumed {
                context.request_redraw();
                return result;
            }
        }
        match event {
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => self.contact(MOUSE, TouchPhase::Started, *position, &geometry, context),
            ViewEvent::PointerMoved { position }
                if self.state.borrow().gestures.contains(MOUSE) =>
            {
                self.contact(MOUSE, TouchPhase::Moved, *position, &geometry, context)
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } if self.state.borrow().gestures.contains(MOUSE) => {
                self.contact(MOUSE, TouchPhase::Ended, *position, &geometry, context)
            }
            ViewEvent::PointerLeft if self.state.borrow().gestures.contains(MOUSE) => self.contact(
                MOUSE,
                TouchPhase::Cancelled,
                Point::default(),
                &geometry,
                context,
            ),
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => {
                if self.state.borrow().editing {
                    self.state.borrow_mut().editing = false;
                } else {
                    let previous = self.state.borrow().previous;
                    self.show(previous, Instant::now());
                }
                context.request_redraw();
                EventResult::Consumed
            }
            ViewEvent::KeyPressed {
                key: Key::Home,
                modifiers,
            } if modifiers.super_key() => {
                self.home(Instant::now(), &geometry);
                context.request_redraw();
                EventResult::Consumed
            }
            ViewEvent::Scroll {
                position,
                delta_x,
                delta_y,
            } => {
                let surface = self.state.borrow().surface;
                if surface == Surface::Notifications {
                    return self.notifications.handle_event(
                        self.notification_bounds(&geometry),
                        event,
                        context,
                    );
                }
                if surface == Surface::Controls {
                    return self.controls.handle_event(bounds, event, context);
                }
                let maximum = self.maximum_scroll(&geometry);
                let app_maximum = self.library_maximum(&geometry);
                let folder_maximum = self.folder_maximum(&geometry);
                let dock_maximum = (self.dock_apps().len() as f32 * geometry.dock_pitch
                    - geometry.dock.size.height
                    + geometry.gap)
                    .max(0.0);
                let mut state = self.state.borrow_mut();
                if surface == Surface::Library {
                    state.app_scroll = (state.app_scroll - *delta_y).clamp(0.0, app_maximum);
                } else if matches!(surface, Surface::Folder(_)) {
                    state.folder_scroll =
                        (state.folder_scroll - *delta_y).clamp(0.0, folder_maximum);
                } else if surface == Surface::Home && geometry.dock.contains(*position) {
                    state.dock_scroll = (state.dock_scroll - *delta_y).clamp(0.0, dock_maximum);
                } else if surface == Surface::Home {
                    let target = model::snap(
                        state.scroll.value - *delta_x - *delta_y,
                        0.,
                        geometry.canvas.size.width,
                        maximum,
                    );
                    state.scroll.settle(target, Instant::now());
                } else {
                    drop(state);
                    return self.foreground_event(&geometry, event, context);
                }
                context.request_redraw();
                EventResult::Consumed
            }
            _ if self.state.borrow().surface == Surface::Notifications => self
                .notifications
                .handle_event(self.notification_bounds(&geometry), event, context),
            _ if self.state.borrow().surface == Surface::Controls => {
                self.controls.handle_event(bounds, event, context)
            }
            _ if self.state.borrow().surface == Surface::Application => {
                self.foreground_event(&geometry, event, context)
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn navigation_bar_follows_drag_then_hides_after_release() {
        let start = Instant::now();
        let mut bar = NavigationBar::default();
        bar.show(false, start);
        let mid_launch = start + Duration::from_millis(45);
        assert!(bar.sample(mid_launch, false).is_some());
        let amount = bar.amount(0.0);
        assert!(amount > 0.0 && amount < 1.0);

        let drag = start + Duration::from_secs(1);
        assert!(bar.sample(drag, false).is_none());
        assert_eq!(bar.amount(0.0), 0.0);
        bar.follow(0.0, -22.0, 44.0);
        assert_eq!(bar.amount(0.0), 0.5);
        bar.follow(0.0, -11.0, 44.0);
        assert_eq!(bar.amount(0.0), 0.25);
        bar.finish_gesture(drag);
        bar.sample(drag, false);
        assert_eq!(bar.amount(0.0), 0.25);

        let deadline = drag + NAVIGATION_REVEAL_TIMEOUT;
        bar.sample(deadline, true);
        assert_eq!(bar.amount(0.0), 1.0);
        bar.sample(deadline, false);
        let mid_hide = deadline + Duration::from_millis(45);
        assert!(bar.sample(mid_hide, false).is_some());
        let value = bar.amount(0.0);
        assert!(value > 0.0 && value < 1.0);
        bar.show(true, mid_hide);
        bar.sample(mid_hide, false);
        assert!((bar.amount(0.0) - value).abs() < 0.0001);
        assert!(
            bar.sample(mid_hide + Duration::from_secs(1), false)
                .is_none()
        );
        assert_eq!(bar.amount(0.0), 1.0);
    }

    #[test]
    fn reversing_application_motion_preserves_sampled_position_and_velocity() {
        let start = Instant::now();
        let mut motion = Motion::default();
        motion.set(720.0);
        motion.animate_application(100.0, start);
        let interrupt = start + Duration::from_millis(55);
        motion.sample(interrupt);
        let (position, velocity) = (motion.value, motion.velocity);
        assert!(position > 100.0 && position < 720.0);
        assert!(velocity < 0.0);
        motion.animate_application(720.0, interrupt);
        motion.sample(interrupt);
        assert!((motion.value - position).abs() < 0.001);
        assert!((motion.velocity - velocity).abs() < 0.001);
        motion.sample(interrupt + Duration::from_secs(2));
        assert!((motion.value - 720.0).abs() < 0.01);
    }

    #[test]
    fn releasing_a_drag_preserves_its_velocity_at_the_handoff() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.follow(0.65, -1.25);
        motion.animate_application(0.0, now);
        motion.sample(now);
        assert!((motion.value - 0.65).abs() < 0.00001);
        assert!((motion.velocity + 1.25).abs() < 0.00001);
    }
}
