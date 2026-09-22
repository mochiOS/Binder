use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::dock_preferences::DockPreferences;
use crate::platform::{self, AppInfo, ContextMenuModel, DesktopPlatform, SystemBarState};
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
    dock_hovered_app: Rc<Cell<Option<usize>>>,
    dock_pressed_app: Rc<Cell<Option<usize>>>,
    dock_pointer: Rc<Cell<Option<Point>>>,
    dock_visibility: Rc<RefCell<crate::ui::dock::DockVisibility>>,
    dock_running_apps: State<Vec<String>>,
    dock_preferences: State<DockPreferences>,
    app_library_open: State<bool>,
    pending_app_activation: Rc<RefCell<crate::ui::app_library::PendingAppActivation>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
    cursor_pointer: Rc<Cell<Option<Point>>>,
    test_window_states: Rc<RefCell<HashMap<WindowId, crate::ui::test::TestWindowState>>>,
    launch_failure_states:
        Rc<RefCell<HashMap<WindowId, crate::ui::launch_failure::LaunchFailureWindowState>>>,
    context_menu: State<Option<ContextMenuModel>>,
    wallpaper: crate::ui::wallpaper::Wallpaper,
    session_user: String,
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
        let context_menu = State::new(None);
        let platform = platform::current(context_menu.clone());
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
            dock_hovered_app: Rc::new(Cell::new(None)),
            dock_pressed_app: Rc::new(Cell::new(None)),
            dock_pointer: Rc::new(Cell::new(None)),
            dock_visibility: Rc::new(RefCell::new(crate::ui::dock::DockVisibility::default())),
            dock_running_apps: State::new(Vec::new()),
            dock_preferences: State::new(DockPreferences::load()),
            app_library_open: State::new(false),
            pending_app_activation: Rc::new(RefCell::new(
                crate::ui::app_library::PendingAppActivation::default(),
            )),
            fast_poll_until: Rc::new(Cell::new(None)),
            cursor_pointer: Rc::new(Cell::new(None)),
            test_window_states: Rc::new(RefCell::new(HashMap::new())),
            launch_failure_states: Rc::new(RefCell::new(HashMap::new())),
            context_menu,
            session_user: crate::session::current_user_label(),
            wallpaper: {
                #[cfg(target_os = "mochios")]
                {
                    crate::ui::wallpaper::Wallpaper::default()
                }
                #[cfg(not(target_os = "mochios"))]
                {
                    crate::ui::wallpaper::Wallpaper::load_default()
                }
            },
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
            Rc::clone(&self.dock_hovered_app),
            Rc::clone(&self.dock_pressed_app),
            Rc::clone(&self.dock_pointer),
            Rc::clone(&self.dock_visibility),
            self.dock_running_apps.clone(),
            self.dock_preferences.clone(),
            self.app_library_open.clone(),
            Rc::clone(&self.pending_app_activation),
            Rc::clone(&self.fast_poll_until),
            Rc::clone(&self.cursor_pointer),
            Rc::clone(&self.test_window_states),
            Rc::clone(&self.launch_failure_states),
            self.context_menu.clone(),
            self.wallpaper.clone(),
            self.session_user.clone(),
        )
    }

    fn handle_platform_message(&mut self, message: &[u8]) -> bool {
        self.platform.borrow_mut().handle_platform_message(message)
    }

    fn appearance_changed(&mut self) {
        let _ = self.platform.borrow_mut().reload_appearance();
    }
}
