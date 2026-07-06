use std::cell::RefCell;
use std::rc::Rc;
use std::time::{
    Duration,
    Instant,
};

use viewkit::{
    prelude::*,
    view::PaintContext,
};

use crate::platform::{
    DesktopPlatform,
    SystemBarState,
};

use super::top_bar;

const DESKTOP_BACKGROUND: Color =
    Color::rgba(200, 200, 200, 255);

const PLATFORM_REFRESH_INTERVAL: Duration =
    Duration::from_secs(1);

pub(crate) fn view(
    system_bar: State<SystemBarState>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    menu_open: State<bool>,
    about_open: State<bool>,
) -> Box<dyn View + 'static> {
    let refresh_driver =
        PlatformRefreshView::new(
            Rc::clone(&platform),
            system_bar.clone(),
        );

    let content = VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::None)
        .child(
            top_bar::view(
                system_bar,
                menu_open.clone(),
            )
                .height(40.0),
        )
        .child(Spacer::new());

    let desktop_content =
        Background::new()
            .background(refresh_driver)
            .content(
                Background::new()
                    .background(
                        Rectangle::new().color(
                            RectangleColor::Custom(
                                DESKTOP_BACKGROUND,
                            ),
                        ),
                    )
                    .content(content),
            );

    let menu =
        super::menu::view(
            platform,
            menu_open.clone(),
            about_open,
        );

    Box::new(
        super::popup_menu::PopupMenu::new(
            desktop_content,
            menu,
            menu_open,
        ),
    )
}

struct PlatformRefreshView {
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    system_bar: State<SystemBarState>,
}

impl PlatformRefreshView {
    fn new(
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        system_bar: State<SystemBarState>,
    ) -> Self {
        Self {
            platform,
            system_bar,
        }
    }
}

impl View for PlatformRefreshView {
    fn paint(
        &self,
        _bounds: Rect,
        context: &mut PaintContext<'_>,
    ) {
        context.request_redraw_at(
            Instant::now()
                + PLATFORM_REFRESH_INTERVAL,
        );

        let changed = {
            let mut platform =
                self.platform.borrow_mut();

            platform.refresh().unwrap_or(false)
        };

        if !changed {
            return;
        }

        let next_state = {
            let platform =
                self.platform.borrow();

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