//! Shell chrome above application windows, with overlap hiding and edge reveal.
use super::{desktop_metrics as metrics, top_bar::SystemBar};
use crate::{
    control_center_preferences::ControlCenterPreferences,
    desktop::WindowResize,
    platform::DesktopPlatform,
    window::{DesktopWindows, WindowDrag},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use viewkit::{
    animation::VisibilityTransition,
    draw_command::PaintTransform,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const REVEAL_EDGE: f32 = 9.0;
const OVERLAP_MARGIN: f32 = 8.0;
const NATIVE_POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct AreaVisibility {
    transition: RefCell<VisibilityTransition>,
    presented: Cell<Option<PaintTransform>>,
    edge_hover: Cell<bool>,
    native_overlap: Cell<Option<(Instant, Rect, bool)>>,
}
#[derive(Default)]
pub(crate) struct StatusAreaInteraction {
    areas: [AreaVisibility; 2],
    context_menu: RefCell<Option<Rc<ContextMenu>>>,
    context_part: Cell<usize>,
    backdrop: Cell<Option<(Instant, Rect, Color)>>,
}

pub(crate) struct StatusAreaLayer<C> {
    content: C,
    bar: SystemBar,
    pub(crate) interaction: Rc<StatusAreaInteraction>,
    windows: State<DesktopWindows>,
    drag: State<Option<WindowDrag>>,
    resize: State<Option<WindowResize>>,
    preferences: Rc<RefCell<ControlCenterPreferences>>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    menu_open: State<bool>,
    controls_open: State<bool>,
    notifications_open: State<bool>,
}
impl<C: View> StatusAreaLayer<C> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        content: C,
        bar: SystemBar,
        interaction: Rc<StatusAreaInteraction>,
        windows: State<DesktopWindows>,
        drag: State<Option<WindowDrag>>,
        resize: State<Option<WindowResize>>,
        preferences: Rc<RefCell<ControlCenterPreferences>>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        menu_open: State<bool>,
        controls_open: State<bool>,
        notifications_open: State<bool>,
    ) -> Self {
        Self {
            content,
            bar,
            interaction,
            windows,
            drag,
            resize,
            preferences,
            platform,
            menu_open,
            controls_open,
            notifications_open,
        }
    }
    fn strip(bounds: Rect) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            metrics::SYSTEM_HEIGHT.min(bounds.size.height),
        )
    }
    fn damage(bounds: Rect) -> Rect {
        let strip = Self::strip(bounds);
        Rect::new(
            strip.origin.x,
            strip.origin.y,
            strip.size.width,
            (strip.size.height + OVERLAP_MARGIN).min(bounds.size.height),
        )
    }
    fn moving_window(&self) -> bool {
        self.drag.get().is_some() || self.resize.get().is_some()
    }
    fn context(&self) -> Option<Rc<ContextMenu>> {
        self.interaction.context_menu.borrow().clone()
    }
    fn regions(&self, bounds: Rect) -> [Rect; 2] {
        self.bar.regions(Self::strip(bounds))
    }
    fn overlaps(&self, bounds: Rect, part: usize) -> bool {
        let region = self.regions(bounds)[part];
        let area = Rect::new(
            region.origin.x,
            region.origin.y,
            region.size.width,
            region.size.height + OVERLAP_MARGIN,
        );
        if self.windows.with(|desktop| {
            desktop
                .windows
                .iter()
                .any(|window| !window.minimized && window.frame.intersection(area).is_some())
        }) {
            return true;
        }
        let cache = &self.interaction.areas[part].native_overlap;
        let now = Instant::now();
        if let Some((at, old_area, overlap)) = cache.get()
            && old_area == area
            && now.saturating_duration_since(at) < NATIVE_POLL
        {
            return overlap;
        }
        let overlap = self
            .platform
            .borrow()
            .native_windows_overlap(area)
            .unwrap_or(false);
        cache.set(Some((now, area, overlap)));
        overlap
    }
    fn should_show(&self, bounds: Rect, part: usize) -> bool {
        let open_menu = if part == 0 {
            self.menu_open.get()
        } else {
            self.controls_open.get() || self.notifications_open.get()
        };
        self.preferences.borrow().always_show_status()
            || self.interaction.areas[part].edge_hover.get()
            || open_menu
            || (self.interaction.context_part.get() == part
                && self.context().is_some_and(|menu| menu.is_open()))
            || !self.overlaps(bounds, part)
    }
    fn update_hover(&self, bounds: Rect, position: Option<Point>) {
        let regions = self.regions(bounds);
        for (part, region) in regions.into_iter().enumerate() {
            let hover = &self.interaction.areas[part].edge_hover;
            let height = if hover.get() {
                metrics::SYSTEM_HEIGHT + OVERLAP_MARGIN
            } else {
                REVEAL_EDGE
            };
            hover.set(
                !self.moving_window()
                    && position.is_some_and(|point| {
                        point.x >= region.origin.x
                            && point.x < region.origin.x + region.size.width
                            && point.y >= bounds.origin.y
                            && point.y <= bounds.origin.y + height
                    }),
            );
        }
    }
    fn update_backdrop(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let region = self.regions(bounds)[1];
        let now = Instant::now();
        let background = if let Some((at, old_region, color)) = self.interaction.backdrop.get()
            && region == old_region
            && now.saturating_duration_since(at) < Duration::from_millis(100)
        {
            color
        } else {
            let background = self
                .platform
                .borrow()
                .native_background_color(region)
                .ok()
                .flatten()
                .unwrap_or_else(|| {
                    context
                        .display_list
                        .sample_background(region, context.theme.shell.window_background)
                });
            self.interaction
                .backdrop
                .set(Some((now, region, background)));
            background
        };
        if self.bar.set_backdrop(background) {
            // A window's damage may cover only half of the pill. Repaint the
            // whole group once when its common foreground changes.
            context.request_redraw_in_at(region, now);
        }
    }
    fn open_context(
        &self,
        bounds: Rect,
        part: usize,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        self.interaction.context_part.set(part);
        let always = self.preferences.borrow().always_show_status();
        let mut menu = Menu::new();
        for (value, title) in [(true, "Always show"), (false, "Hide when windows overlap")] {
            let preferences = self.preferences.clone();
            menu = menu.item(
                MenuItem::new(if value == always {
                    format!("✓ {title}")
                } else {
                    title.into()
                })
                .on_select(move || {
                    let mut preferences = preferences.borrow_mut();
                    preferences.set_always_show_status(value);
                    if let Err(error) = preferences.save() {
                        eprintln!("failed to save StatusArea setting: {error}");
                    }
                }),
            );
        }
        let menu = Rc::new(ContextMenu::new(Spacer::new(), menu));
        let result = menu.handle_event(bounds, event, context);
        *self.interaction.context_menu.borrow_mut() = Some(menu);
        result
    }
}
impl<C: View> View for StatusAreaLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        self.update_backdrop(bounds, context);
        for part in 0..2 {
            let visible = self.should_show(bounds, part);
            let state = &self.interaction.areas[part];
            let (amount, next) = state.transition.borrow_mut().sample(
                visible,
                context.theme.motion.standard,
                Instant::now(),
            );
            if let Some(next) = next {
                context.request_redraw_in_at(Self::damage(bounds), next);
            }
            if amount > 0.0 {
                let transform = PaintTransform {
                    anchor: bounds.origin,
                    scale: 1.0,
                    offset: Point::new(0.0, -(1.0 - amount) * metrics::SYSTEM_HEIGHT),
                    opacity: amount,
                };
                state.presented.set(Some(transform));
                context.with_presentation(transform, visible, |context| {
                    self.bar.paint_part(part, Self::strip(bounds), context)
                });
            } else {
                state.presented.set(None);
            }
        }
        if let Some(menu) = self.context()
            && menu.is_open()
        {
            menu.paint(bounds, context);
        }
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let old_hover = self
            .interaction
            .areas
            .each_ref()
            .map(|state| state.edge_hover.get());
        match event {
            ViewEvent::PointerMoved { position }
            | ViewEvent::PointerPressed { position, .. }
            | ViewEvent::PointerReleased { position, .. } => {
                self.update_hover(bounds, Some(*position))
            }
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                self.update_hover(bounds, None)
            }
            _ => {}
        }
        if old_hover
            != self
                .interaction
                .areas
                .each_ref()
                .map(|state| state.edge_hover.get())
        {
            context.request_redraw_in(Self::damage(bounds));
        }
        if let Some(menu) = self.context()
            && menu.is_open()
        {
            return menu
                .handle_event(bounds, event, context)
                .merge(EventResult::Consumed);
        }
        if self.moving_window() {
            return self.content.handle_event(bounds, event, context);
        }
        for part in 0..2 {
            if self.should_show(bounds, part) {
                if event.position().is_none()
                    && self.interaction.areas[part].presented.get().is_some()
                {
                    let result = self
                        .bar
                        .handle_part(part, Self::strip(bounds), event, context);
                    if result.is_consumed() {
                        return result;
                    }
                }
                if let Some(transform) = self.interaction.areas[part].presented.get() {
                    let logical = super::popup_menu::logical_event(event, transform);
                    if logical
                        .position()
                        .is_some_and(|point| self.regions(bounds)[part].contains(point))
                    {
                        if let ViewEvent::PointerPressed {
                            position,
                            button: PointerButton::Secondary,
                        } = &logical
                            && self.bar.is_blank(Self::strip(bounds), *position)
                        {
                            return self.open_context(bounds, part, event, context);
                        }
                        context.set_cursor(CursorIcon::Default);
                        return self
                            .bar
                            .handle_part(part, Self::strip(bounds), &logical, context)
                            .merge(EventResult::Consumed);
                    }
                }
            }
            self.bar
                .handle_part(part, Self::strip(bounds), &ViewEvent::PointerLeft, context);
        }
        self.content.handle_event(bounds, event, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{PlatformError, ProcessId, SystemAction, SystemBarState};
    use viewkit::{draw_command::DisplayList, typography::TextMeasurer};
    struct Platform;
    impl DesktopPlatform for Platform {
        fn system_bar_state(&self) -> Result<SystemBarState, PlatformError> {
            Ok(SystemBarState::default())
        }
        fn refresh(&mut self) -> Result<bool, PlatformError> {
            Ok(false)
        }
        fn open_system_settings(&self) -> Result<(), PlatformError> {
            Ok(())
        }
        fn perform_system_action(&self, _: SystemAction) -> Result<(), PlatformError> {
            Ok(())
        }
    }
    struct WindowProbe(Rc<Cell<usize>>);
    impl View for WindowProbe {
        fn measure(&self, c: Constraints, _: &mut MeasureContext<'_>) -> Size {
            c.maximum
        }
        fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
            Text::new("WINDOW-CONTENT").paint(bounds, context);
        }
        fn handle_event(&self, _: Rect, _: &ViewEvent, _: &mut EventContext<'_>) -> EventResult {
            self.0.set(self.0.get() + 1);
            EventResult::Consumed
        }
    }
    fn layer() -> StatusAreaLayer<WindowProbe> {
        let controls = State::new(false);
        let notifications = State::new(false);
        let menu = State::new(false);
        let preferences = Rc::new(RefCell::new(ControlCenterPreferences::default()));
        let bar = super::super::top_bar::view(
            State::new(SystemBarState::default()),
            menu.clone(),
            controls.clone(),
            notifications.clone(),
            State::new(Vec::new()),
            preferences.clone(),
            Rc::new(RefCell::new(
                super::super::control_center::ControlCenterInteraction::default(),
            )),
        );
        StatusAreaLayer::new(
            WindowProbe(Rc::new(Cell::new(0))),
            bar,
            Rc::new(StatusAreaInteraction::default()),
            State::new(DesktopWindows::default()),
            State::new(None),
            State::new(None),
            preferences,
            Rc::new(RefCell::new(Platform)),
            menu,
            controls,
            notifications,
        )
    }
    fn overlap(layer: &StatusAreaLayer<WindowProbe>) {
        layer.windows.update(|desktop| {
            desktop.open_test(ProcessId(1), "Test".into(), 1280, 800, true);
            desktop.windows[0].frame.origin = Point::new(0.0, 0.0);
        });
    }
    #[test]
    fn right_edge_reveal_and_open_menus_do_not_reveal_the_opposite_side() {
        let layer = layer();
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        overlap(&layer);
        assert!(!layer.should_show(bounds, 0));
        assert!(!layer.should_show(bounds, 1));
        layer.update_hover(bounds, Some(Point::new(1100.0, 3.0)));
        assert!(layer.should_show(bounds, 1));
        assert!(!layer.should_show(bounds, 0));
        assert!(!layer.menu_open.get());
        layer.update_hover(bounds, None);
        layer.controls_open.set(true);
        assert!(layer.should_show(bounds, 1));
        assert!(!layer.should_show(bounds, 0));
        layer.controls_open.set(false);
        layer.menu_open.set(true);
        assert!(layer.should_show(bounds, 0));
        assert!(!layer.should_show(bounds, 1));
    }

    #[test]
    fn overlap_hides_edge_reveals_and_drag_does_not_capture_the_window() {
        let layer = layer();
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        assert!(layer.should_show(bounds, 1));
        overlap(&layer);
        assert!(!layer.should_show(bounds, 1));
        layer.update_hover(bounds, Some(Point::new(1100.0, 20.0)));
        assert!(!layer.should_show(bounds, 1));
        layer.update_hover(bounds, Some(Point::new(1100.0, 3.0)));
        assert!(layer.should_show(bounds, 1));
        layer.update_hover(bounds, Some(Point::new(1100.0, 35.0)));
        assert!(layer.should_show(bounds, 1));
        let window = layer.windows.get().windows[0].clone();
        layer.drag.set(Some(WindowDrag {
            window: window.id,
            pointer_origin: window.frame.origin,
            window_origin: window.frame.origin,
        }));
        layer.update_hover(bounds, Some(Point::new(1100.0, 3.0)));
        assert!(!layer.should_show(bounds, 1));
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        layer.handle_event(
            bounds,
            &ViewEvent::PointerMoved {
                position: Point::new(1100.0, 3.0),
            },
            &mut context,
        );
        assert_eq!(layer.content.0.get(), 1);
        layer.drag.set(None);
        layer.controls_open.set(true);
        assert!(layer.should_show(bounds, 1));
        layer.controls_open.set(false);
        layer.preferences.borrow_mut().set_always_show_status(true);
        assert!(layer.should_show(bounds, 1));
        layer.preferences.borrow_mut().set_always_show_status(false);
        layer
            .windows
            .update(|desktop| desktop.windows[0].minimized = true);
        assert!(layer.should_show(bounds, 1));
    }
    #[test]
    fn hidden_chrome_does_not_take_focus_or_input_and_edge_reveal_restores_it() {
        let layer = layer();
        overlap(&layer);
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut theme = Theme::current();
        theme.motion = theme.motion.reduced();
        let mut measurer = TextMeasurer::new();
        let mut list = DisplayList::new();
        let mut nodes = Vec::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes),
        );
        assert!(layer.interaction.areas[1].presented.get().is_none());
        assert!(
            !nodes
                .iter()
                .any(|node| node.label.as_deref() == Some("Control Center"))
        );
        {
            let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
            layer.handle_event(
                bounds,
                &ViewEvent::PointerPressed {
                    position: Point::new(1235.0, 30.0),
                    button: PointerButton::Primary,
                },
                &mut context,
            );
        }
        assert_eq!(layer.content.0.get(), 1);
        layer.update_hover(bounds, Some(Point::new(1235.0, 3.0)));
        list.clear();
        nodes.clear();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes),
        );
        assert!(layer.interaction.areas[1].presented.get().is_some());
        assert!(
            nodes
                .iter()
                .any(|node| node.label.as_deref() == Some("Control Center"))
        );
    }
    #[test]
    fn native_context_menu_switches_and_persists_visibility_without_losing_its_open_state() {
        let layer = layer();
        let path =
            std::env::temp_dir().join(format!("mochios-status-area-{}.conf", std::process::id()));
        *layer.preferences.borrow_mut() = ControlCenterPreferences::with_test_storage(path.clone());
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut theme = Theme::current();
        theme.motion = theme.motion.reduced();
        let mut measurer = TextMeasurer::new();
        let mut list = DisplayList::new();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
        );
        // The overlay paints after application content.
        assert!(
            matches!(&list.commands()[0], viewkit::draw_command::DrawCommand::DrawText { command } if command.text == "WINDOW-CONTENT")
        );
        // A completed reveal must retain the status pill's original 64/255
        // alpha, rather than turning it into an opaque white cover.
        assert!(list.commands().iter().any(|command| matches!(command,
            viewkit::draw_command::DrawCommand::FillRoundedRect { rect, color, .. }
                if rect.size.width == metrics::STATUS_GROUP_WIDTH && rect.size.height == 34.0
                    && *color == Color::rgba(255, 255, 255, 64))));
        assert!(!list.commands().iter().any(|command| matches!(command,
            viewkit::draw_command::DrawCommand::FillRoundedRect { rect, color, .. }
                if rect.size.width == metrics::STATUS_GROUP_WIDTH && rect.size.height == 34.0
                    && color.alpha == 255)));
        {
            let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
            layer.handle_event(
                bounds,
                &ViewEvent::PointerPressed {
                    position: Point::new(1235.0, 25.0),
                    button: PointerButton::Secondary,
                },
                &mut context,
            );
        }
        let menu = layer.context().unwrap();
        assert!(menu.is_open());
        let mut nodes = Vec::new();
        list.clear();
        layer.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer)
                .with_accessibility_nodes(&mut nodes),
        );
        let item = nodes
            .iter()
            .find(|node| node.label.as_deref() == Some("Always show"))
            .unwrap()
            .bounds;
        let position = Point::new(
            item.origin.x + item.size.width * 0.5,
            item.origin.y + item.size.height * 0.5,
        );
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        layer.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        layer.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert!(layer.preferences.borrow().always_show_status());
        assert!(!menu.is_open());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("status-area=always")
        );
        std::fs::remove_file(path).unwrap();
    }
}
