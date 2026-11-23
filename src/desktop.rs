use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::control_center_preferences::ControlCenterPreferences;
use crate::dock_preferences::DockPreferences;
use crate::platform::{self, AppInfo, ContextMenuModel, DesktopPlatform, SystemBarState};
use crate::window::{DesktopWindows, WindowDrag, WindowId};
use viewkit::prelude::*;

pub struct BinderApp {
    shell_mode: crate::shell::ShellMode,
    app_icons: crate::ui::AppIcons,
    tablet: Rc<RefCell<crate::ui::tablet::TabletState>>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    system_bar: State<SystemBarState>,
    mochios_menu_open: State<bool>,
    menu_interaction: Rc<crate::ui::popup_menu::PopupMenuInteraction>,
    control_center_open: State<bool>,
    notification_center_open: State<bool>,
    notification_center_interaction:
        Rc<crate::ui::notification_center::NotificationCenterInteraction>,
    control_center_preferences: Rc<RefCell<ControlCenterPreferences>>,
    control_center_interaction: Rc<RefCell<crate::ui::control_center::ControlCenterInteraction>>,
    control_center_editor_interaction:
        Rc<RefCell<crate::ui::control_center_editor::ControlCenterEditorInteraction>>,
    windows: State<DesktopWindows>,
    window_drag: State<Option<WindowDrag>>,
    window_resize: State<Option<WindowResize>>,
    apps: State<Vec<AppInfo>>,
    dock_hovered_app: Rc<Cell<Option<usize>>>,
    dock_pressed_app: Rc<Cell<Option<usize>>>,
    dock_pointer: Rc<Cell<Option<Point>>>,
    dock_visibility: Rc<RefCell<crate::ui::dock::DockVisibility>>,
    status_area_interaction: Rc<crate::ui::status_area::StatusAreaInteraction>,
    dock_running_apps: State<Vec<String>>,
    dock_preferences: State<DockPreferences>,
    app_library_open: State<bool>,
    app_switcher: Rc<RefCell<crate::ui::app_switcher::AppSwitcherState>>,
    spotlight: Rc<RefCell<crate::ui::spotlight::SpotlightState>>,
    pending_app_activation: Rc<RefCell<crate::ui::app_library::PendingAppActivation>>,
    fast_poll_until: Rc<Cell<Option<Instant>>>,
    cursor_pointer: Rc<Cell<Option<Point>>>,
    cursor_initialized: Rc<Cell<bool>>,
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
        let mut startup = crate::ui::app_library::PendingAppActivation::default();
        if let Some(app) = crate::startup::application(&apps) {
            startup.queue(app);
        }
        Self {
            shell_mode: crate::shell::ShellMode::current(),
            app_icons: crate::ui::AppIcons::default(),
            tablet: Rc::new(RefCell::new(crate::ui::tablet::TabletState::default())),
            platform,
            system_bar: State::new(system_bar),
            mochios_menu_open: State::new(false),
            menu_interaction: Rc::new(crate::ui::popup_menu::PopupMenuInteraction::default()),
            control_center_open: State::new(false),
            notification_center_open: State::new(false),
            notification_center_interaction: Rc::new(
                crate::ui::notification_center::NotificationCenterInteraction::default(),
            ),
            control_center_preferences: Rc::new(RefCell::new(ControlCenterPreferences::load())),
            control_center_interaction: Rc::new(RefCell::new(
                crate::ui::control_center::ControlCenterInteraction::default(),
            )),
            control_center_editor_interaction: Rc::new(RefCell::new(
                crate::ui::control_center_editor::ControlCenterEditorInteraction::default(),
            )),
            windows: State::new(DesktopWindows::default()),
            window_drag: State::new(None),
            window_resize: State::new(None),
            apps: State::new(apps),
            dock_hovered_app: Rc::new(Cell::new(None)),
            dock_pressed_app: Rc::new(Cell::new(None)),
            dock_pointer: Rc::new(Cell::new(None)),
            dock_visibility: Rc::new(RefCell::new(crate::ui::dock::DockVisibility::default())),
            status_area_interaction: Rc::new(
                crate::ui::status_area::StatusAreaInteraction::default(),
            ),
            dock_running_apps: State::new(Vec::new()),
            dock_preferences: State::new(DockPreferences::load()),
            app_library_open: State::new(false),
            app_switcher: Rc::new(RefCell::new(
                crate::ui::app_switcher::AppSwitcherState::default(),
            )),
            spotlight: Rc::new(RefCell::new(crate::ui::spotlight::SpotlightState::default())),
            pending_app_activation: Rc::new(RefCell::new(startup)),
            fast_poll_until: Rc::new(Cell::new(None)),
            cursor_pointer: Rc::new(Cell::new(None)),
            cursor_initialized: Rc::new(Cell::new(false)),
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
        #[cfg(not(target_os = "linux"))]
        let options = WindowOptions::new("Binder").size(1280.0, 800.0);
        #[cfg(target_os = "linux")]
        {
            let title = match self.shell_mode {
                crate::shell::ShellMode::Desktop => {
                    "Binder - Ctrl + Alt + G: Release mouse | Click inside: Capture mouse"
                }
                crate::shell::ShellMode::Tablet => "Binder - Tablet",
            };
            WindowOptions::new(title)
                .size(1280.0, 800.0)
                .resizable(true)
                .fullscreen(self.shell_mode == crate::shell::ShellMode::Tablet)
        }
        #[cfg(not(target_os = "linux"))]
        {
            options.resizable(false).fullscreen(true)
        }
    }

    fn first_frame_presented(&mut self) {
        self.platform.borrow_mut().first_frame_presented();
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        if self.shell_mode == crate::shell::ShellMode::Tablet {
            return Box::new(crate::ui::tablet::TabletShell::new(
                crate::ui::tablet::Resources {
                    platform: self.platform.clone(),
                    system_bar: self.system_bar.clone(),
                    windows: self.windows.clone(),
                    apps: self.apps.clone(),
                    running: self.dock_running_apps.clone(),
                    dock: self.dock_preferences.clone(),
                    pending: self.pending_app_activation.clone(),
                    fast_poll: self.fast_poll_until.clone(),
                    test_states: self.test_window_states.clone(),
                    failures: self.launch_failure_states.clone(),
                    control_preferences: self.control_center_preferences.clone(),
                    control_interaction: self.control_center_interaction.clone(),
                    control_editor: self.control_center_editor_interaction.clone(),
                    wallpaper: self.wallpaper.clone(),
                },
                self.tablet.clone(),
            ));
        }
        crate::ui::desktop::view(
            self.system_bar.clone(),
            self.status_area_interaction.clone(),
            Rc::clone(&self.platform),
            self.mochios_menu_open.clone(),
            Rc::clone(&self.menu_interaction),
            self.control_center_open.clone(),
            self.notification_center_open.clone(),
            Rc::clone(&self.notification_center_interaction),
            Rc::clone(&self.control_center_preferences),
            Rc::clone(&self.control_center_interaction),
            Rc::clone(&self.control_center_editor_interaction),
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
            Rc::clone(&self.app_switcher),
            Rc::clone(&self.spotlight),
            Rc::clone(&self.pending_app_activation),
            Rc::clone(&self.fast_poll_until),
            Rc::clone(&self.cursor_pointer),
            Rc::clone(&self.cursor_initialized),
            Rc::clone(&self.test_window_states),
            Rc::clone(&self.launch_failure_states),
            self.context_menu.clone(),
            self.wallpaper.clone(),
            self.session_user.clone(),
            self.app_icons.clone(),
        )
    }

    fn handle_platform_message(&mut self, message: &[u8]) -> bool {
        self.platform.borrow_mut().handle_platform_message(message)
    }

    fn appearance_changed(&mut self) {
        #[cfg(target_os = "linux")]
        {
            self.wallpaper = crate::ui::wallpaper::Wallpaper::load_default();
        }
        let _ = self.platform.borrow_mut().reload_appearance();
    }
}
