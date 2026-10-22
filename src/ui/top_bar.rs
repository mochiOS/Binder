use std::sync::OnceLock;

use super::desktop_metrics as metrics;
use crate::platform::SystemBarState;
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) fn view(
    state: State<SystemBarState>,
    menu_open: State<bool>,
    control_open: State<bool>,
    notification_open: State<bool>,
) -> impl View + 'static {
    let menu = menu_open.clone();
    let controls = control_open.clone();
    let notifications = notification_open.clone();
    let logo = Button::new("")
        .content(Logo)
        .style(ButtonStyle::Ghost)
        .accessibility_label("mochiOS menu")
        .on_click(move || {
            menu.set(!menu.get());
            controls.set(false);
            notifications.set(false);
        });
    let notifications = notification_open.clone();
    let controls = control_open.clone();
    let menu = menu_open.clone();
    let notification_button = Button::new("")
        .content(
            Icon::new(SymbolName::Bell)
                .size(20.0)
                .color(metrics::SYSTEM_FOREGROUND),
        )
        .style(ButtonStyle::Ghost)
        .accessibility_label("Notification Center")
        .accessibility_value(format!("{} unread", state.get().notifications.unread_count))
        .on_click(move || {
            notifications.set(!notifications.get());
            controls.set(false);
            menu.set(false);
        });
    let input_state = state.clone();
    let input = Button::new("")
        .content(InputLanguage(state.clone()))
        .style(ButtonStyle::Ghost)
        .accessibility_label("Switch input language")
        .on_click(move || {
            if let Some(enabled) = viewkit::platform::input_method::toggle() {
                input_state.update(|state| state.japanese_input = enabled);
            }
        });
    let menu = menu_open;
    let controls = control_open;
    let status = Button::new("")
        .content(StatusContent)
        .style(ButtonStyle::Ghost)
        .accessibility_label("Control Center")
        .on_click(move || {
            controls.set(!controls.get());
            menu.set(false);
            notification_open.set(false);
        });
    SystemBar {
        logo,
        status,
        input,
        notifications: notification_button,
        clock: Clock(state),
    }
}

struct SystemBar {
    logo: Button,
    status: Button,
    input: Button,
    notifications: Button,
    clock: Clock,
}
fn logo_frame(bounds: Rect) -> Rect {
    Rect::new(
        bounds.origin.x + metrics::LOGO_INSET - 10.0,
        bounds.origin.y + metrics::LOGO_INSET - 10.0,
        44.0,
        44.0,
    )
}
impl View for SystemBar {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(constraints.maximum.width, metrics::SYSTEM_HEIGHT))
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.logo.paint(logo_frame(bounds), context);
        Rectangle::new()
            .color(RectangleColor::Custom(Color::rgba(255, 255, 255, 64)))
            .radius(CornerRadius::Full)
            .paint(status_frame(bounds), context);
        self.status.paint(controls_frame(bounds), context);
        self.input.paint(input_frame(bounds), context);
        self.notifications
            .paint(status_content_frames(status_frame(bounds))[2], context);
        self.clock
            .paint(status_content_frames(status_frame(bounds))[0], context);
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let input = self.input.handle_event(input_frame(bounds), event, context);
        if input == EventResult::Consumed {
            return input;
        }
        let notifications = self.notifications.handle_event(
            status_content_frames(status_frame(bounds))[2],
            event,
            context,
        );
        if notifications == EventResult::Consumed {
            return notifications;
        }
        self.logo
            .handle_event(logo_frame(bounds), event, context)
            .merge(
                self.status
                    .handle_event(controls_frame(bounds), event, context),
            )
    }
}
fn controls_frame(bounds: Rect) -> Rect {
    let group = status_frame(bounds);
    Rect::new(
        group.origin.x + 164.0,
        group.origin.y,
        group.size.width - 164.0,
        group.size.height,
    )
}
fn input_frame(bounds: Rect) -> Rect {
    let frame = status_content_frames(status_frame(bounds))[1];
    Rect::new(
        frame.origin.x - 4.0,
        frame.origin.y,
        frame.size.width + 8.0,
        frame.size.height,
    )
}
fn status_frame(bounds: Rect) -> Rect {
    Rect::new(
        bounds.origin.x + bounds.size.width - metrics::CHROME_RIGHT - metrics::STATUS_GROUP_WIDTH,
        bounds.origin.y + metrics::STATUS_GROUP_TOP,
        metrics::STATUS_GROUP_WIDTH,
        34.0,
    )
}
// Every item shares the pill's full height and exact center; intrinsic ink is centered by ViewKit.
fn status_content_frames(bounds: Rect) -> [Rect; 5] {
    [
        Rect::new(
            bounds.origin.x + 8.0,
            bounds.origin.y,
            104.0,
            bounds.size.height,
        ),
        Rect::new(
            bounds.origin.x + 112.0,
            bounds.origin.y,
            16.0,
            bounds.size.height,
        ),
        Rect::new(
            bounds.origin.x + 136.0,
            bounds.origin.y,
            24.0,
            bounds.size.height,
        ),
        Rect::new(
            bounds.origin.x + 168.0,
            bounds.origin.y,
            32.0,
            bounds.size.height,
        ),
        Rect::new(
            bounds.origin.x + 208.0,
            bounds.origin.y,
            24.0,
            bounds.size.height,
        ),
    ]
}
struct StatusContent;
impl View for StatusContent {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let frames = status_content_frames(Rect::new(
            bounds.origin.x - 164.0,
            bounds.origin.y,
            metrics::STATUS_GROUP_WIDTH,
            bounds.size.height,
        ));
        Icon::new(SymbolName::BatteryFull)
            .size(28.0)
            .color(metrics::SYSTEM_FOREGROUND)
            .accessibility_label("Battery")
            .paint(frames[3], context);
        Icon::new(SymbolName::Wifi)
            .size(20.0)
            .color(metrics::SYSTEM_FOREGROUND)
            .accessibility_label("Network")
            .paint(frames[4], context);
    }
}
struct Logo;
impl View for Logo {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(metrics::LOGO_SIZE, metrics::LOGO_SIZE))
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        static LOGO: OnceLock<SvgData> = OnceLock::new();
        let logo = LOGO.get_or_init(|| {
            SvgData::decode(include_bytes!("../../resources/mochios.svg")).expect("mochiOS logo")
        });
        Svg::new(logo.clone()).paint(
            Rect::new(
                bounds.origin.x + (bounds.size.width - metrics::LOGO_SIZE) * 0.5,
                bounds.origin.y + (bounds.size.height - metrics::LOGO_SIZE) * 0.5,
                metrics::LOGO_SIZE,
                metrics::LOGO_SIZE,
            ),
            context,
        );
    }
}
fn label(
    value: impl Into<String>,
    bounds: Rect,
    alignment: TextAlignment,
    center_visible_ink: bool,
    context: &mut PaintContext<'_>,
) {
    let height = 20.0 * context.text_measurer.font_scale();
    let text_bounds = if center_visible_ink {
        bounds
    } else {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + (bounds.size.height - height) * 0.5,
            bounds.size.width,
            height,
        )
    };
    Text::styled(value, TextRole::Label)
        .font_size(14.0)
        .line_height(20.0)
        .weight(400)
        .alignment(alignment)
        .color(metrics::SYSTEM_FOREGROUND)
        .center_visible_ink(center_visible_ink)
        .paint(text_bounds, context);
}
struct InputLanguage(State<SystemBarState>);
impl View for InputLanguage {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        label(
            if self.0.get().japanese_input {
                "あ"
            } else {
                "A"
            },
            bounds,
            TextAlignment::Center,
            true,
            context,
        );
    }
}
struct Clock(State<SystemBarState>);
impl View for Clock {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let clock = self.0.get().clock;
        label(
            format!("{}, {}", short_date(&clock.date), clock.time),
            bounds,
            TextAlignment::Start,
            false,
            context,
        );
    }
}
fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if let Some((month, day)) = date
        .split_whitespace()
        .next()
        .and_then(|value| value.split_once('/'))
        && let (Ok(month), Ok(day)) = (month.parse::<usize>(), day.parse::<u32>())
        && let Some(month) = month.checked_sub(1).and_then(|index| MONTHS.get(index))
    {
        return format!("{day} {month}");
    }
    date.to_owned()
}

#[cfg(test)]
mod alignment_tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use viewkit::platform::PointerButton;
    use viewkit::typography::TextMeasurer;

    #[test]
    fn notification_bell_is_independent_and_date_is_reserved_for_calendar() {
        let menu = State::new(false);
        let controls = State::new(false);
        let notifications = State::new(false);
        let bar = view(
            State::new(SystemBarState::default()),
            menu,
            controls.clone(),
            notifications.clone(),
        );
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let frames = status_content_frames(status_frame(bounds));
        for (index, notification_open, control_open) in
            [(0, false, false), (2, true, false), (4, false, true)]
        {
            let frame = frames[index];
            let position = Point::new(
                frame.origin.x + frame.size.width * 0.5,
                frame.origin.y + frame.size.height * 0.5,
            );
            bar.handle_event(bounds, &ViewEvent::PointerMoved { position }, &mut context);
            bar.handle_event(
                bounds,
                &ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                },
                &mut context,
            );
            bar.handle_event(
                bounds,
                &ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                },
                &mut context,
            );
            assert_eq!(notifications.get(), notification_open);
            assert_eq!(controls.get(), control_open);
        }
    }

    #[test]
    fn input_language_click_does_not_open_control_center() {
        let toggles = Rc::new(Cell::new(0));
        let controls = Rc::new(Cell::new(0));
        let toggle_calls = toggles.clone();
        let control_calls = controls.clone();
        let bar = SystemBar {
            logo: Button::new(""),
            notifications: Button::new(""),
            clock: Clock(State::new(SystemBarState::default())),
            input: Button::new("").on_click(move || toggle_calls.set(toggle_calls.get() + 1)),
            status: Button::new("").on_click(move || control_calls.set(control_calls.get() + 1)),
        };
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let bounds = Rect::new(0.0, 0.0, 1440.0, 54.0);
        for position in [Point::new(1310.0, 27.0), Point::new(1410.0, 27.0)] {
            bar.handle_event(bounds, &ViewEvent::PointerMoved { position }, &mut context);
            bar.handle_event(
                bounds,
                &ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                },
                &mut context,
            );
            bar.handle_event(
                bounds,
                &ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                },
                &mut context,
            );
        }
        assert_eq!(toggles.get(), 1);
        assert_eq!(controls.get(), 1);
    }
    #[test]
    fn status_items_share_the_pill_center_at_any_origin_and_height() {
        for bounds in [
            Rect::new(1190.0, 10.0, 235.0, 34.0),
            Rect::new(80.0, 53.5, 235.0, 51.0),
        ] {
            let center = bounds.origin.y + bounds.size.height * 0.5;
            for frame in status_content_frames(bounds) {
                assert_eq!(frame.origin.y + frame.size.height * 0.5, center);
            }
        }
    }
}
