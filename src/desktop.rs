use std::cell::RefCell;
use std::rc::Rc;

use viewkit::prelude::*;
use crate::platform::{self, DesktopPlatform, SystemBarState};

pub struct BinderApp {
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    system_bar: State<SystemBarState>,
}

impl App for BinderApp {
    type Body = Box<dyn View + 'static>;

    fn new() -> Self {
        let platform = platform::current();

        let system_bar = platform.borrow().system_bar_state().unwrap_or_default();

        Self {
            platform,
            system_bar: State::new(system_bar),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Binder")
            .size(1280.0, 800.0)
            .resizable(false)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        crate::ui::desktop::view(self.system_bar.get(), self.platform.clone())
    }
}
