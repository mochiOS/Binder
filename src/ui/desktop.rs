use std::cell::RefCell;
use std::rc::Rc;

use super::top_bar;
use crate::desktop::WindowResize;
use crate::platform::{ApplicationId, DesktopPlatform, SystemBarState};
use crate::window::{DesktopWindows, WindowDrag};
use std::time::{Duration, Instant};
use viewkit::{prelude::*, view::PaintContext};

const DESKTOP_BACKGROUND: Color = Color::rgba(200, 200, 200, 255);
const PLATFORM_REFRESH_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) fn view(
    system_bar: State<SystemBarState>,

    platform: Rc<RefCell<dyn DesktopPlatform>>,

    menu_open: State<bool>,
    windows: State<DesktopWindows>,

    window_drag: State<Option<WindowDrag>>,

    resize: State<Option<WindowResize>>,
) -> Box<dyn View + 'static> {
    let refresh_driver =
        PlatformRefreshView::new(Rc::clone(&platform), system_bar.clone(), windows.clone());

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

    let menu = super::menu::view(platform, menu_open.clone(), windows);

    Box::new(super::popup_menu::PopupMenu::new(
        windowed_desktop,
        menu,
        menu_open,
    ))
}

struct PlatformRefreshView {
    platform: Rc<RefCell<dyn DesktopPlatform>>,

    system_bar: State<SystemBarState>,

    windows: State<DesktopWindows>,
}

impl PlatformRefreshView {
    fn new(
        platform: Rc<RefCell<dyn DesktopPlatform>>,

        system_bar: State<SystemBarState>,

        windows: State<DesktopWindows>,
    ) -> Self {
        Self {
            platform,
            system_bar,
            windows,
        }
    }
}

impl View for PlatformRefreshView {
    fn paint(&self, _bounds: Rect, context: &mut PaintContext<'_>) {
        context.request_redraw_at(Instant::now() + PLATFORM_REFRESH_INTERVAL);

        let active_processes = {
            let desktop = self.windows.get();

            desktop.process_ids()
        };

        let (system_bar_changed, create_window_requests, exited_processes) = {
            let mut platform = self.platform.borrow_mut();

            if let Err(error) = platform.synchronize_applications(&active_processes) {
                eprintln!("failed to synchronize applications: {error:?}",);
            }

            let system_bar_changed = platform.refresh().unwrap_or_else(|error| {
                eprintln!("failed to refresh platform: {error:?}",);

                false
            });

            let create_window_requests = platform.take_create_window_requests();

            let exited_processes = platform.take_exited_processes();

            (system_bar_changed, create_window_requests, exited_processes)
        };

        if !create_window_requests.is_empty() || !exited_processes.is_empty() {
            self.windows.update(|desktop| {
                for request in create_window_requests {
                    match request.application {
                        ApplicationId::About => {
                            desktop.open_about(request.process_id);
                        }
                    }
                }

                for process_id in exited_processes {
                    desktop.close_process(process_id);
                }
            });
        }

        if !system_bar_changed {
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
        }
    }
}
