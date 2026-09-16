use std::cell::RefCell;
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform, SystemBarState};
use crate::window::DesktopWindows;
use viewkit::prelude::*;
use viewkit::view::PaintContext;

const BAR_CONTENT_HEIGHT: f32 = 39.0;
const HORIZONTAL_PADDING: f32 = 14.0;
const VERTICAL_PADDING: f32 = 5.0;
const CLOCK_WIDTH: f32 = 220.0;
const CLOCK_HEIGHT: f32 = 29.0;

pub(crate) fn view(
    system_bar: State<SystemBarState>,
    menu_open: State<bool>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
) -> impl View + 'static {
    let menu_open_on_click = menu_open.clone();

    let menu_button = Button::new("")
        .content(
            Text::styled("mochiOS", TextRole::Caption)
                .weight(650)
                .color(Theme::current().shell.primary_text)
                .height(Theme::current().typography.caption.line_height),
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
        .child(ActiveApplicationName::new(platform, windows, apps).width(180.0))
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
                .background(Rectangle::new().color(RectangleColor::Custom(
                    Theme::current().shell.bar_background,
                )))
                .content(Padding::symmetric(HORIZONTAL_PADDING, VERTICAL_PADDING).content(row))
                .height(BAR_CONTENT_HEIGHT),
        )
        .child(Divider::new())
}

struct ActiveApplicationName {
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
}

impl ActiveApplicationName {
    fn new(
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
    ) -> Self {
        Self {
            platform,
            windows,
            apps,
        }
    }

    fn application_name(&self) -> String {
        let process_id = self.windows.get().focused_process_id();
        let apps = self.apps.get();
        let platform = self.platform.borrow();

        process_id
            .and_then(|process_id| {
                apps.iter()
                    .find(|app| platform.process_id_for_bundle(&app.bundle_id) == Some(process_id))
                    .map(|app| app.name.clone())
            })
            .unwrap_or_else(|| String::from("Binder"))
    }
}

impl View for ActiveApplicationName {
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Text::styled(self.application_name(), TextRole::Caption)
            .weight(650)
            .color(Theme::current().shell.primary_text)
            .paint(bounds, context);
    }
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
                    .color(Theme::current().shell.primary_text)
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
                    .color(Theme::current().shell.secondary_text)
                    .frame(CLOCK_WIDTH, 11.0),
            );
        }

        content.paint(bounds, context);
    }
}
