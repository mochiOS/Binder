use crate::platform::SystemBarState;
use viewkit::prelude::*;
use viewkit::view::PaintContext;

const BAR_CONTENT_HEIGHT: f32 = 39.0;
const HORIZONTAL_PADDING: f32 = 14.0;
const VERTICAL_PADDING: f32 = 5.0;
const MENU_HEIGHT: f32 = 29.0;
const CLOCK_WIDTH: f32 = 220.0;
const CLOCK_HEIGHT: f32 = 29.0;

const BAR_BACKGROUND: Color = Color::rgba(250, 250, 250, 100);

const PRIMARY_TEXT: Color = Color::from_rgb_hex(0x191919);

const SECONDARY_TEXT: Color = Color::from_rgb_hex(0x626262);

pub(crate) fn view(
    system_bar: State<SystemBarState>,
    menu_open: State<bool>,
) -> impl View + 'static {
    let menu_open_on_click = menu_open.clone();

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
            let next = !menu_open_on_click.get();

            menu_open_on_click.set(next);
        });

    let leading = HStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::None)
        .child(menu_button)
        .child(Spacer::new());

    let clock = SystemBarClock::new(system_bar);

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

struct SystemBarClock {
    system_bar: State<SystemBarState>,
}

impl SystemBarClock {
    fn new(system_bar: State<SystemBarState>) -> Self {
        Self { system_bar }
    }
}

impl View for SystemBarClock {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let state = self.system_bar.get();

        let time = state.clock.time;

        let date = state.clock.date;

        let mut content = VStack::new()
            .alignment(StackAlignment::Center)
            .distribution(StackDistribution::Center)
            .gap(StackGap::Custom(1.0));

        if !time.is_empty() {
            content = content.child(
                Text::new(time)
                    .font_size(12.0)
                    .line_height(15.0)
                    .weight(700)
                    .alignment(TextAlignment::Center)
                    .color(PRIMARY_TEXT)
                    .frame(CLOCK_WIDTH, 15.0),
            );
        }

        if !date.is_empty() {
            content = content.child(
                Text::new(date)
                    .font_size(9.0)
                    .line_height(12.0)
                    .weight(600)
                    .alignment(TextAlignment::Center)
                    .color(SECONDARY_TEXT)
                    .frame(CLOCK_WIDTH, 11.0),
            );
        }

        content.paint(bounds, context);
    }
}
