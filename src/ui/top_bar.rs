use std::cell::RefCell;
use std::rc::Rc;

use viewkit::prelude::*;

use crate::platform::{ClockState, DesktopPlatform, SystemBarState};

const BAR_CONTENT_HEIGHT: f32 = 39.0;

const HORIZONTAL_PADDING: f32 = 14.0;
const VERTICAL_PADDING: f32 = 5.0;
const MENU_WIDTH: f32 = 92.0;
const MENU_HEIGHT: f32 = 29.0;
const CLOCK_WIDTH: f32 = 220.0;
const CLOCK_HEIGHT: f32 = 29.0;

const BAR_BACKGROUND: Color = Color::rgba(250, 250, 250, 100);

const PRIMARY_TEXT: Color = Color::from_rgb_hex(0x191919);

const SECONDARY_TEXT: Color = Color::from_rgb_hex(0x626262);

pub(crate) fn view(
    system_bar: SystemBarState,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
) -> impl View + 'static {
    let ClockState { date, time } = system_bar.clock;

    let menu_platform = Rc::clone(&platform);

    let menu_button = Button::new("")
        .content(
            Text::new("mochiOS")
                .font_size(12.0)
                .line_height(18.0)
                .weight(650)
                .color(PRIMARY_TEXT)
                .height(18.0),
        )
        .style(ButtonStyle::Ghost)
        .alignment(ZStackAlignment::Leading)
        .on_click(move || {
            let result = menu_platform
                .borrow()
                .open_system_menu();

            if let Err(error) = result {
                eprintln!(
                    "failed to open mochiOS menu: {error:?}",
                );
            }
        })
        .frame(
            MENU_WIDTH,
            MENU_HEIGHT,
        );

    let leading = HStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::None)
        .child(menu_button)
        .child(Spacer::new());

    let clock = clock_view(date, time);

    let trailing = HStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::None);

    let row = HStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::None)
        .child(leading.layout().width(0.0).flex_grow(1.0))
        .child(clock.frame(CLOCK_WIDTH, CLOCK_HEIGHT).flex_shrink(0.0))
        .child(trailing.layout().width(0.0).flex_grow(1.0));

    VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::None)
        .child(
            Background::new()
                .background(Rectangle::new().color(RectangleColor::Custom(BAR_BACKGROUND)))
                .content(Padding::symmetric(HORIZONTAL_PADDING, VERTICAL_PADDING).content(row))
                .height(BAR_CONTENT_HEIGHT),
        )
        .child(Divider::new())
}

fn clock_view(
    date: String,
    time: String,
) -> VStack {
    let mut clock = VStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::Custom(1.0));

    if !time.is_empty() {
        clock = clock.child(
            Text::new(time)
                .font_size(12.0)
                .line_height(15.0)
                .weight(700)
                .alignment(TextAlignment::Center)
                .color(PRIMARY_TEXT)
                .frame(
                    CLOCK_WIDTH,
                    15.0,
                ),
        );
    }

    if !date.is_empty() {
        clock = clock.child(
            Text::new(date)
                .font_size(9.0)
                .line_height(12.0)
                .weight(600)
                .alignment(TextAlignment::Center)
                .color(SECONDARY_TEXT)
                .frame(
                    CLOCK_WIDTH,
                    11.0,
                ),
        );
    }

    clock
}
