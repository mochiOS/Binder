use std::cell::RefCell;
use std::rc::Rc;

use viewkit::prelude::*;

use crate::platform::{DesktopPlatform, SystemBarState};

use super::top_bar;

const DESKTOP_BACKGROUND: Color = Color::rgba(100, 100, 100, 255);

pub(crate) fn view(
    system_bar: SystemBarState,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
) -> Box<dyn View + 'static> {
    let content = VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::None)
        .child(top_bar::view(system_bar, platform).height(40.0))
        .child(Spacer::new());

    Box::new(
        Background::new()
            .background(Rectangle::new().color(RectangleColor::Custom(DESKTOP_BACKGROUND)))
            .content(content),
    )
}
