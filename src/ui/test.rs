use viewkit::prelude::*;

#[derive(Clone)]
pub(crate) struct TestWindowState {
    text: TextFieldInteractionState,
    scroll: ScrollState,
}

impl Default for TestWindowState {
    fn default() -> Self {
        Self {
            text: TextFieldInteractionState::new(),
            scroll: ScrollState::new(),
        }
    }
}

pub(crate) fn view(state: TestWindowState) -> impl View + 'static {
    let rows = VStack::new()
        .alignment(StackAlignment::Stretch)
        .gap(StackGap::Custom(6.0))
        .children((1..=24).map(|index| {
            Text::new(format!("GPU accelerated row {index:02}"))
                .font_size(13.0)
                .line_height(20.0)
                .height(24.0)
        }));

    Padding::all(18.0).content(
        VStack::new()
            .alignment(StackAlignment::Stretch)
            .gap(StackGap::Medium)
            .child(TextField::with_interaction(state.text).placeholder("Type in this window"))
            .child(
                Scroll::new(state.scroll)
                    .axis(ScrollAxis::Vertical)
                    .scrollbar(ScrollBarVisibility::Automatic)
                    .content(rows)
                    .layout()
                    .flex_grow(1.0),
            ),
    )
}
