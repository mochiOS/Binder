use viewkit::prelude::*;

const CONTENT_BACKGROUND: Color = Color::from_rgb_hex(0xFAFAFA);

const PRIMARY_TEXT: Color = Color::from_rgb_hex(0x171717);

const SECONDARY_TEXT: Color = Color::from_rgb_hex(0x686868);

pub(crate) fn view() -> impl View + 'static {
    Background::new()
        .background(Rectangle::new().color(RectangleColor::Custom(CONTENT_BACKGROUND)))
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
                            .color(PRIMARY_TEXT),
                    )
                    .child(
                        Text::new("26.0 Kinako")
                            .font_size(13.0)
                            .line_height(20.0)
                            .alignment(TextAlignment::Center)
                            .color(SECONDARY_TEXT),
                    )
                    .child(
                        Text::new("Developing")
                            .font_size(12.0)
                            .line_height(18.0)
                            .alignment(TextAlignment::Center)
                            .color(SECONDARY_TEXT),
                    ),
            ),
        )
}
