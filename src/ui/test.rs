use viewkit::prelude::*;

pub(crate) fn view() -> impl View + 'static {
    VStack::new()
        .alignment(StackAlignment::Center)
        .distribution(StackDistribution::Center)
        .gap(StackGap::Small)
        .child(Text::new("TestWindow"))
}
