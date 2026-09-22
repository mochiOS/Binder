use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use std::time::{Duration, Instant};

use super::{dock, top_bar};

use crate::desktop::WindowResize;
use crate::dock_preferences::DockPreferences;

use crate::platform::ContextMenuModel;
use crate::platform::{AppInfo, DesktopPlatform, ProcessId, RemoteWindowId, SystemBarState};

use crate::window::{DesktopWindows, WindowDrag, WindowId};

use crate::apps;
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const PLATFORM_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const LAUNCH_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const SYSTEM_BAR_HEIGHT: f32 = 40.0;
const DOCK_DAMAGE_HEIGHT: f32 = 150.0;
const WINDOW_EFFECT_EXTENT: f32 = 20.0;

pub(crate) fn view(
    system_bar: State<SystemBarState>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    menu_open: State<bool>,
    windows: State<DesktopWindows>,
    window_drag: State<Option<WindowDrag>>,
    resize: State<Option<WindowResize>>,
    apps: State<Vec<AppInfo>>,
    dock_hovered: Rc<Cell<Option<usize>>>,
    dock_pressed: Rc<Cell<Option<usize>>>,
    dock_pointer: Rc<Cell<Option<Point>>>,
    dock_visibility: Rc<RefCell<dock::DockVisibility>>,
    dock_running_apps: State<Vec<String>>,
    dock_preferences: State<DockPreferences>,
    app_library_open: State<bool>,
    pending_app_activation: Rc<RefCell<super::app_library::PendingAppActivation>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
    cursor_pointer: Rc<std::cell::Cell<Option<Point>>>,
    test_window_states: Rc<RefCell<HashMap<WindowId, super::test::TestWindowState>>>,
    launch_failure_states: Rc<
        RefCell<HashMap<WindowId, super::launch_failure::LaunchFailureWindowState>>,
    >,
    context_menu: State<Option<ContextMenuModel>>,
    wallpaper: super::wallpaper::Wallpaper,
    session_user: String,
) -> Box<dyn View + 'static> {
    let refresh_driver = PlatformRefreshView::new(
        Rc::clone(&platform),
        system_bar.clone(),
        windows.clone(),
        apps.clone(),
        dock_running_apps.clone(),
        Rc::clone(&fast_poll_until),
    );

    let content = VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::None)
        .child(
            top_bar::view(
                system_bar,
                menu_open.clone(),
                Rc::clone(&platform),
                windows.clone(),
                apps.clone(),
            )
            .height(40.0),
        )
        .child(Spacer::new());

    let desktop_content = Background::new().background(refresh_driver).content(
        Background::new()
            .background(
                Background::new()
                    .background(Rectangle::new().color(RectangleColor::Custom(Color::TRANSPARENT)))
                    .content(wallpaper),
            )
            .content(content),
    );

    let windowed_desktop = super::window_layer::WindowLayer::new(
        desktop_content,
        windows.clone(),
        window_drag,
        resize,
        test_window_states,
        Rc::clone(&launch_failure_states),
    );

    let docked_desktop = dock::DockLayer::new(
        windowed_desktop,
        Rc::clone(&platform),
        windows.clone(),
        apps.clone(),
        dock_hovered,
        dock_pressed,
        dock_pointer,
        Rc::clone(&cursor_pointer),
        dock_visibility,
        dock_running_apps,
        dock_preferences.clone(),
        app_library_open.clone(),
        Rc::clone(&fast_poll_until),
        Rc::clone(&launch_failure_states),
    );

    let menu = super::menu::view(
        Rc::clone(&platform),
        menu_open.clone(),
        windows.clone(),
        session_user,
    );

    let root = super::app_library::AppLibraryLayer::new(
        docked_desktop,
        Rc::clone(&platform),
        windows.clone(),
        apps,
        dock_preferences,
        app_library_open,
        pending_app_activation,
        fast_poll_until,
        launch_failure_states,
    );
    let root = super::popup_menu::PopupMenu::new(root, menu, menu_open);
    let root = super::context_menu::ContextMenuLayer::new(root, Rc::clone(&platform), context_menu);
    let root = PointerTracker::new(root, Rc::clone(&cursor_pointer));

    #[cfg(target_os = "mochios")]
    {
        Box::new(root)
    }

    #[cfg(not(target_os = "mochios"))]
    {
        Box::new(super::cursor::CursorLayer::new(root, cursor_pointer))
    }
}

struct PointerTracker<C> {
    content: C,
    position: Rc<Cell<Option<Point>>>,
}

impl<C: View> PointerTracker<C> {
    fn new(content: C, position: Rc<Cell<Option<Point>>>) -> Self {
        Self { content, position }
    }
}

impl<C: View> View for PointerTracker<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
    }

    fn handle_event(&self, bounds: Rect, event: &ViewEvent, context: &mut EventContext<'_>) -> EventResult {
        let next = match event {
            ViewEvent::PointerMoved { position }
            | ViewEvent::PointerPressed { position, .. }
            | ViewEvent::PointerReleased { position, .. } => Some(Some(*position)),
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => Some(None),
            _ => None,
        };
        if let Some(next) = next {
            let previous = self.position.replace(next);
            if previous != next {
                let dock_zone = Rect::new(
                    bounds.origin.x,
                    bounds.origin.y + (bounds.size.height - 190.0).max(0.0),
                    bounds.size.width,
                    bounds.size.height.min(190.0),
                );
                if previous.is_some_and(|point| dock_zone.contains(point))
                    || next.is_some_and(|point| dock_zone.contains(point))
                {
                    context.request_redraw_in(dock_zone);
                }
            }
        }
        self.content.handle_event(bounds, event, context)
    }
}

#[cfg(test)]
mod pointer_tracker_tests {
    use super::*;

    struct ConsumingView;

    impl View for ConsumingView {
        fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
            constraints.maximum
        }

        fn paint(&self, _bounds: Rect, _context: &mut PaintContext<'_>) {}

        fn handle_event(&self, _bounds: Rect, _event: &ViewEvent, _context: &mut EventContext<'_>) -> EventResult {
            EventResult::Consumed
        }
    }

    #[test]
    fn tracks_pointer_even_when_child_consumes_motion() {
        let position = Rc::new(Cell::new(None));
        let layer = PointerTracker::new(ConsumingView, Rc::clone(&position));
        let theme = Theme::current();
        let mut measurer = viewkit::typography::TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let result = layer.handle_event(
            Rect::new(0.0, 0.0, 1280.0, 800.0),
            &ViewEvent::PointerMoved { position: Point::new(640.0, 790.0) },
            &mut context,
        );
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(position.get(), Some(Point::new(640.0, 790.0)));
    }
}

struct PlatformRefreshView {
    platform: Rc<RefCell<dyn DesktopPlatform>>,

    system_bar: State<SystemBarState>,

    windows: State<DesktopWindows>,

    apps: State<Vec<AppInfo>>,

    dock_running_apps: State<Vec<String>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
    last_refresh: Cell<Option<Instant>>,
}

impl PlatformRefreshView {
    fn new(
        platform: Rc<RefCell<dyn DesktopPlatform>>,

        system_bar: State<SystemBarState>,

        windows: State<DesktopWindows>,

        apps: State<Vec<AppInfo>>,

        dock_running_apps: State<Vec<String>>,
        fast_poll_until: Rc<Cell<Option<Instant>>>,
    ) -> Self {
        Self {
            platform,
            system_bar,
            windows,
            apps,
            dock_running_apps,
            fast_poll_until,
            last_refresh: Cell::new(None),
        }
    }

    fn refresh_interval(&self) -> Duration {
        if self.fast_poll_until.get().is_some_and(|until| Instant::now() < until) {
            LAUNCH_REFRESH_INTERVAL
        } else {
            PLATFORM_REFRESH_INTERVAL
        }
    }

    fn system_bar_damage(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            SYSTEM_BAR_HEIGHT.min(bounds.size.height),
        )
    }

    fn dock_damage(bounds: Rect) -> Rect {
        let height = DOCK_DAMAGE_HEIGHT.min(bounds.size.height);
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height - height,
            bounds.size.width,
            height,
        )
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

    fn window_change_damage(before: Option<Rect>, after: Option<Rect>) -> Option<Rect> {
        match (before, after) {
            (Some(before), Some(after)) => Some(before.union(after)),
            (Some(dirty), None) | (None, Some(dirty)) => Some(dirty),
            (None, None) => None,
        }
    }
}

impl View for PlatformRefreshView {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let poll_wake_region = Rect::new(bounds.origin.x, bounds.origin.y, 1.0, 1.0);
        let needs_notifications = {
            let desktop = self.windows.get();

            desktop.has_pending_platform_notifications()
        };

        let now = Instant::now();
        let interval = self.refresh_interval();
        if !needs_notifications
            && let Some(last) = self.last_refresh.get()
            && let Some(next) = last.checked_add(interval)
            && now < next
        {
            context.request_redraw_in_at(poll_wake_region, next);
            return;
        }
        self.last_refresh.set(Some(now));

        let (pending_close_requests, resized_notifications, focus_notifications) =
            if needs_notifications {
                let mut close_requests = Vec::new();

                let mut resized = Vec::new();

                let mut focus_changed = Vec::new();

                self.windows.update(|desktop| {
                    close_requests = desktop.take_pending_close_requests();

                    (resized, focus_changed) = desktop.take_window_state_notifications();
                });

                (close_requests, resized, focus_changed)
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };

        let active_processes = {
            let desktop = self.windows.get();

            desktop.process_ids()
        };

        let (
            system_bar_changed,
            create_requests,
            close_requests,
            exited_processes,
            failed_close_requests,
            running_apps,
            discovered_apps,
        ) = {
            let mut platform = self.platform.borrow_mut();

            let mut failed_close_requests = Vec::new();

            for request in pending_close_requests {
                if let Err(error) =
                    platform.request_window_close(request.process_id, request.window)
                {
                    eprintln!("failed to request window close: {error:?}",);

                    failed_close_requests.push(request);
                }
            }

            for notification in resized_notifications {
                if let Err(error) = platform.notify_window_resized(notification) {
                    eprintln!("failed to notify window resize: {error:?}",);
                }
            }

            for notification in focus_notifications {
                if let Err(error) = platform.notify_window_focus_changed(notification) {
                    eprintln!("failed to notify window focus: {error:?}",);
                }
            }

            if let Err(error) = platform.synchronize_applications(&active_processes) {
                eprintln!("failed to synchronize applications: {error:?}",);
            }

            let system_bar_changed = platform.refresh().unwrap_or_else(|error| {
                eprintln!("failed to refresh platform: {error:?}",);

                false
            });

            let running_apps = platform.running_app_bundle_ids();

            let discovered_apps = platform.get_apps();

            let create_requests = platform.take_create_window_requests();

            let close_requests = platform.take_close_window_requests();

            let exited_processes = platform.take_exited_processes();

            (
                system_bar_changed,
                create_requests,
                close_requests,
                exited_processes,
                failed_close_requests,
                running_apps,
                discovered_apps,
            )
        };

        if self.apps.get() != discovered_apps {
            self.apps.set(discovered_apps);
            context.request_redraw_in_at(Self::dock_damage(bounds), Instant::now());
        }

        if self.dock_running_apps.get() != running_apps {
            self.dock_running_apps.set(running_apps);
            context.request_redraw_in_at(Self::dock_damage(bounds), Instant::now());
        }

        let has_window_changes = !create_requests.is_empty()
            || !close_requests.is_empty()
            || !exited_processes.is_empty()
            || !failed_close_requests.is_empty();

        let mut registrations: Vec<(ProcessId, RemoteWindowId)> = Vec::new();

        if has_window_changes {
            let before = self.visible_windows_damage();
            let work_area = super::window_layer::window_work_area(bounds);
            self.windows.update(|desktop| {
                for request in create_requests {
                    match request.renderer.as_str() {
                        apps::ABOUT_ENTRY => {
                            let (_window_id, remote_window) = desktop.open_about(
                                request.process_id,
                                request.title,
                                request.width,
                                request.height,
                                request.resizable,
                            );

                            registrations.push((request.process_id, remote_window));
                        }

                        apps::TEST_ENTRY => {
                            let (_window_id, remote_window) = desktop.open_test(
                                request.process_id,
                                request.title,
                                request.width,
                                request.height,
                                request.resizable,
                            );

                            registrations.push((request.process_id, remote_window));
                        }

                        _ => {
                            let (_window_id, remote_window) = desktop.open_window(
                                request.process_id,
                                request.renderer,
                                request.title,
                                request.width,
                                request.height,
                                request.resizable,
                            );

                            registrations.push((request.process_id, remote_window));
                        }
                    }
                }

                for request in close_requests {
                    desktop.close_remote(request.process_id, request.window);
                }

                for process_id in exited_processes {
                    desktop.close_process(process_id);
                }

                for request in failed_close_requests {
                    desktop.cancel_close_request(request.process_id, request.window);
                }
                // Clamp a newly spawned frame before registering its remote
                // surface or rendering its first visible frame.
                desktop.fit_to_work_area(work_area);
            });
            if let Some(dirty) = Self::window_change_damage(before, self.visible_windows_damage()) {
                context.request_redraw_in_at(dirty, Instant::now());
            }
        }

        let mut failed_registrations = Vec::new();

        if !registrations.is_empty() {
            let mut platform = self.platform.borrow_mut();

            for (process_id, remote_window) in registrations {
                if let Err(error) = platform.register_window(process_id, remote_window) {
                    eprintln!("failed to register window: {error:?}",);

                    failed_registrations.push(process_id);
                }
            }
        }

        if !failed_registrations.is_empty() {
            let before = self.visible_windows_damage();
            self.windows.update(|desktop| {
                for process_id in failed_registrations {
                    desktop.close_process(process_id);
                }
            });
            if let Some(dirty) = Self::window_change_damage(before, self.visible_windows_damage()) {
                context.request_redraw_in_at(dirty, Instant::now());
            }
        }

        if !system_bar_changed {
            context
                .request_redraw_in_at(poll_wake_region, Instant::now() + self.refresh_interval());
            return;
        }

        let next_state = {
            let platform = self.platform.borrow();

            platform.system_bar_state()
        };

        let Ok(next_state) = next_state else {
            return;
        };

        if self.system_bar.get() != next_state {
            self.system_bar.set(next_state);
            context.request_redraw_in_at(Self::system_bar_damage(bounds), Instant::now());
        }

        context.request_redraw_in_at(poll_wake_region, Instant::now() + self.refresh_interval());
    }
}
