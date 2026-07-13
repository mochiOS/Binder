use std::cell::RefCell;
use std::rc::Rc;

use std::time::{Duration, Instant};

use super::{dock, top_bar};

use crate::desktop::WindowResize;

use crate::platform::{AppInfo, DesktopPlatform, ProcessId, RemoteWindowId, SystemBarState};

use crate::window::{DesktopWindows, WindowDrag};

use crate::apps;
use viewkit::{prelude::*, view::PaintContext};

const DESKTOP_BACKGROUND: Color = Color::rgba(200, 200, 200, 255);

const PLATFORM_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) fn view(
    system_bar: State<SystemBarState>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    menu_open: State<bool>,
    windows: State<DesktopWindows>,
    window_drag: State<Option<WindowDrag>>,
    resize: State<Option<WindowResize>>,
    apps: State<Vec<AppInfo>>,
    dock_hovered: State<Option<usize>>,
    dock_pressed: State<Option<usize>>,
    dock_pointer: State<Option<Point>>,
    dock_running_apps: State<Vec<String>>,
    cursor_pointer: Rc<std::cell::Cell<Option<Point>>>,
) -> Box<dyn View + 'static> {
    let refresh_driver = PlatformRefreshView::new(
        Rc::clone(&platform),
        system_bar.clone(),
        windows.clone(),
        dock_running_apps.clone(),
    );

    let content = VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::None)
        .child(top_bar::view(system_bar, menu_open.clone()).height(40.0))
        .child(Spacer::new());

    let desktop_content = Background::new().background(refresh_driver).content(
        Background::new()
            .background(Rectangle::new().color(RectangleColor::Custom(DESKTOP_BACKGROUND)))
            .content(content),
    );

    let windowed_desktop = super::window_layer::WindowLayer::new(
        desktop_content,
        windows.clone(),
        window_drag,
        resize,
    );

    let docked_desktop = dock::DockLayer::new(
        windowed_desktop,
        Rc::clone(&platform),
        windows.clone(),
        apps,
        dock_hovered,
        dock_pressed,
        dock_pointer,
        dock_running_apps,
    );

    let menu = super::menu::view(platform, menu_open.clone(), windows);

    let root = super::popup_menu::PopupMenu::new(docked_desktop, menu, menu_open);

    #[cfg(target_os = "mochios")]
    {
        let _ = cursor_pointer;
        Box::new(root)
    }

    #[cfg(not(target_os = "mochios"))]
    {
        Box::new(super::cursor::CursorLayer::new(root, cursor_pointer))
    }
}

struct PlatformRefreshView {
    platform: Rc<RefCell<dyn DesktopPlatform>>,

    system_bar: State<SystemBarState>,

    windows: State<DesktopWindows>,

    dock_running_apps: State<Vec<String>>,
}

impl PlatformRefreshView {
    fn new(
        platform: Rc<RefCell<dyn DesktopPlatform>>,

        system_bar: State<SystemBarState>,

        windows: State<DesktopWindows>,

        dock_running_apps: State<Vec<String>>,
    ) -> Self {
        Self {
            platform,
            system_bar,
            windows,
            dock_running_apps,
        }
    }
}

impl View for PlatformRefreshView {
    fn paint(&self, _bounds: Rect, context: &mut PaintContext<'_>) {
        let needs_notifications = {
            let desktop = self.windows.get();

            desktop.has_pending_platform_notifications()
        };

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
            )
        };

        if self.dock_running_apps.get() != running_apps {
            self.dock_running_apps.set(running_apps);
            context.request_redraw_at(Instant::now());
        }

        let has_window_changes = !create_requests.is_empty()
            || !close_requests.is_empty()
            || !exited_processes.is_empty()
            || !failed_close_requests.is_empty();

        let mut registrations: Vec<(ProcessId, RemoteWindowId)> = Vec::new();

        if has_window_changes {
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
            });
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
            self.windows.update(|desktop| {
                for process_id in failed_registrations {
                    desktop.close_process(process_id);
                }
            });
            context.request_redraw_at(Instant::now());
        }

        if !system_bar_changed {
            context.request_redraw_at(Instant::now() + PLATFORM_REFRESH_INTERVAL);
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
            context.request_redraw_at(Instant::now());
        }

        context.request_redraw_at(Instant::now() + PLATFORM_REFRESH_INTERVAL);
    }
}
