use std::cell::RefCell;
use std::rc::Rc;

use viewkit::prelude::*;
use crate::platform::{self, DesktopPlatform, SystemBarState};

pub struct BinderApp {
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    system_bar: State<SystemBarState>,
    mochios_menu_open: State<bool>,
    about_open: State<bool>,
}

impl App for BinderApp {
    type Body = Box<dyn View + 'static>;

    fn new() -> Self {
        let platform = platform::current();

        let system_bar = platform.borrow().system_bar_state().unwrap_or_default();

        Self {
            platform,
            system_bar: State::new(system_bar),
            mochios_menu_open: State::new(false),
            about_open: State::new(false),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Binder")
            .size(1280.0, 800.0)
            .resizable(false)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        crate::ui::desktop::view(
            self.system_bar.clone(),
            Rc::clone(&self.platform),
            self.mochios_menu_open.clone(),
            self.about_open.clone(),
        )
    }
}
