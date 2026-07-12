use std::cell::RefCell;
use std::rc::Rc;

use crate::platform::{self, AppInfo, DesktopPlatform, SystemBarState};
use crate::window::{DesktopWindows, WindowDrag, WindowId};
use viewkit::prelude::*;

pub struct BinderApp {
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    system_bar: State<SystemBarState>,
    mochios_menu_open: State<bool>,
    windows: State<DesktopWindows>,
    window_drag: State<Option<WindowDrag>>,
    window_resize: State<Option<WindowResize>>,
    apps: State<Vec<AppInfo>>,
    dock_hovered_app: State<Option<usize>>,
    dock_pressed_app: State<Option<usize>>,
    dock_pointer: State<Option<Point>>,
    dock_running_apps: State<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowResize {
    pub window: WindowId,
    pub edge: ResizeEdge,
    pub pointer_origin: Point,
    pub frame_origin: Point,
    pub frame_size: Size,
}
impl App for BinderApp {
    type Body = Box<dyn View + 'static>;

    fn new() -> Self {
        let platform = platform::current();
        let system_bar = platform.borrow().system_bar_state().unwrap_or_default();
        let apps = platform.borrow().get_apps();

        Self {
            platform,
            system_bar: State::new(system_bar),
            mochios_menu_open: State::new(false),
            windows: State::new(DesktopWindows::default()),
            window_drag: State::new(None),
            window_resize: State::new(None),
            apps: State::new(apps),
            dock_hovered_app: State::new(None),
            dock_pressed_app: State::new(None),
            dock_pointer: State::new(None),
            dock_running_apps: State::new(Vec::new()),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Binder")
            .size(1280.0, 800.0)
            .resizable(false)
            .fullscreen(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        crate::ui::desktop::view(
            self.system_bar.clone(),
            Rc::clone(&self.platform),
            self.mochios_menu_open.clone(),
            self.windows.clone(),
            self.window_drag.clone(),
            self.window_resize.clone(),
            self.apps.clone(),
            self.dock_hovered_app.clone(),
            self.dock_pressed_app.clone(),
            self.dock_pointer.clone(),
            self.dock_running_apps.clone(),
        )
    }
}
