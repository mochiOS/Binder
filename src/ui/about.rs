use viewkit::prelude::*;

pub(crate) fn view() -> impl View + 'static {
    Background::new()
        .background(Rectangle::new().color(RectangleColor::Custom(
            Theme::current().shell.content_background,
        )))
        .content(
            Padding::all(28.0).content(
                VStack::new()
                    .alignment(StackAlignment::Center)
                    .distribution(StackDistribution::Center)
                    .gap(StackGap::Custom(8.0))
                    .child(
                        Text::new("mochiOS")
                            .font_size(32.0)
                            .line_height(40.0)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.primary_text),
                    )
                    .child(
                        Text::new("26.0 Kinako")
                            .font_size(13.0)
                            .line_height(20.0)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.secondary_text),
                    )
                    .child(
                        Text::new("Developing")
                            .font_size(12.0)
                            .line_height(18.0)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.secondary_text),
                    ),
            ),
        )
}
