use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use super::desktop_metrics as metrics;
use crate::platform::{DesktopPlatform, SystemBarState};
use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

pub(crate) fn view(
    state: State<SystemBarState>,
    menu_open: State<bool>,
    control_open: State<bool>,
    notifications_open: State<bool>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
) -> impl View + 'static {
    let (menu, control, notifications) = (
        menu_open.clone(),
        control_open.clone(),
        notifications_open.clone(),
    );
    let logo = Button::new("")
        .content(Logo)
        .style(ButtonStyle::Ghost)
        .accessibility_label("mochiOS system menu")
        .on_click(move || {
            let next = !menu.get();
            control.set(false);
            notifications.set(false);
            menu.set(next);
        });
    let (menu, control, notifications) = (
        menu_open.clone(),
        control_open.clone(),
        notifications_open.clone(),
    );
    let clock = Button::new("")
        .content(Clock(state.clone()))
        .style(ButtonStyle::Ghost)
        .alignment(ZStackAlignment::Leading)
        .accessibility_label("Date, time and notifications")
        .on_click(move || {
            let next = !notifications.get();
            menu.set(false);
            control.set(false);
            notifications.set(next);
            if next {
                let _ = platform.borrow_mut().mark_all_notifications_read();
            }
        });
    let input_state = state.clone();
    let input = Button::new("")
        .content(InputLanguage(state))
        .style(ButtonStyle::Ghost)
        .accessibility_label("Switch input language")
        .on_click(move || {
            let current = viewkit::platform::input_method::enabled()
                .unwrap_or_else(|| input_state.get().japanese_input);
            let enabled = viewkit::platform::input_method::toggle().unwrap_or(!current);
            input_state.update_if_changed(|state| state.japanese_input = enabled);
        });
    let status_button = |symbol, size, label| {
        let (menu, control, notifications) = (
            menu_open.clone(),
            control_open.clone(),
            notifications_open.clone(),
        );
        Button::new("")
            .content(
                Icon::new(symbol)
                    .size(size)
                    .color(metrics::SYSTEM_FOREGROUND),
            )
            .style(ButtonStyle::Ghost)
            .accessibility_label(label)
            .on_click(move || {
                let next = !control.get();
                menu.set(false);
                notifications.set(false);
                control.set(next);
            })
    };
    SystemBar {
        buttons: [
            logo,
            clock,
            input,
            status_button(SymbolName::BatteryFull, 28.0, "Battery and system controls"),
            status_button(SymbolName::Wifi, 20.0, "Network and system controls"),
        ],
    }
}

struct SystemBar {
    buttons: [Button; 5],
}
impl SystemBar {
    fn frames(bounds: Rect) -> [Rect; 5] {
        let status_x =
            bounds.origin.x + bounds.size.width - metrics::STATUS_RIGHT - metrics::STATUS_WIDTH;
        let y = bounds.origin.y + metrics::LOGO_INSET;
        [
            Rect::new(
                bounds.origin.x + metrics::LOGO_INSET - 10.0,
                y - 10.0,
                44.0,
                44.0,
            ),
            Rect::new(status_x, y, 96.0, 24.0),
            Rect::new(status_x + 96.0, y, 16.0, 24.0),
            Rect::new(status_x + 120.0, y - 5.0, 32.0, 34.0),
            Rect::new(status_x + 160.0, y, 24.0, 24.0),
        ]
    }
}
impl View for SystemBar {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(constraints.maximum.width, metrics::SYSTEM_HEIGHT))
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        for (button, frame) in self.buttons.iter().zip(Self::frames(bounds)) {
            button.paint(frame, context);
        }
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let mut result = EventResult::Ignored;
        for (button, frame) in self.buttons.iter().zip(Self::frames(bounds)) {
            result = result.merge(button.handle_event(frame, event, context));
        }
        result
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
    context: &mut PaintContext<'_>,
) {
    let height = 20.0 * context.text_measurer.font_scale();
    Text::styled(value, TextRole::Label)
        .font_size(14.0)
        .line_height(20.0)
        .weight(400)
        .alignment(alignment)
        .color(metrics::SYSTEM_FOREGROUND)
        .paint(
            Rect::new(
                bounds.origin.x,
                bounds.origin.y + (bounds.size.height - height) * 0.5,
                bounds.size.width,
                height,
            ),
            context,
        );
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
