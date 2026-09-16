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
                        Text::styled("mochiOS", TextRole::TitleLarge)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.primary_text),
                    )
                    .child(
                        Text::styled("26.0 Kinako", TextRole::Label)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.secondary_text),
                    )
                    .child(
                        Text::styled("Developing", TextRole::Caption)
                            .alignment(TextAlignment::Center)
                            .color(Theme::current().shell.secondary_text),
                    ),
            ),
        )
}
