use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const CURSOR_SVG_PATH: &str = "/system/icons/cursor.svg";
const CURSOR_SVG_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/system/icons/cursor.svg"
));
const CURSOR_WIDTH: f32 = 16.0;
const CURSOR_HEIGHT: f32 = 27.0;
const CURSOR_HOTSPOT_X: f32 = 1.0;
const CURSOR_HOTSPOT_Y: f32 = 1.0;

pub(crate) struct CursorLayer<C> {
    content: C,
    pointer: Rc<Cell<Option<Point>>>,
    initialized: Rc<Cell<bool>>,
}

impl<C> CursorLayer<C>
where
    C: View,
{
    pub(crate) fn new(
        content: C,
        pointer: Rc<Cell<Option<Point>>>,
        initialized: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            content,
            pointer,
            initialized,
        }
    }

    fn position_for_paint(&self, bounds: Rect) -> Option<Point> {
        if !self.initialized.replace(true) && self.pointer.get().is_none() {
            self.pointer.set(Some(Point::new(
                bounds.origin.x + bounds.size.width / 2.0,
                bounds.origin.y + bounds.size.height / 2.0,
            )));
        }
        self.pointer.get()
    }

    fn cursor_image() -> Option<ImageData> {
        static CURSOR: OnceLock<Option<ImageData>> = OnceLock::new();

        CURSOR
            .get_or_init(|| {
                let svg = SvgData::from_path(CURSOR_SVG_PATH)
                    .or_else(|_| SvgData::decode(CURSOR_SVG_BYTES))
                    .ok()?;
                ImageData::from_svg(&svg, CURSOR_WIDTH as u32, CURSOR_HEIGHT as u32).ok()
            })
            .clone()
    }

    fn cursor_bounds(pointer: Point) -> Rect {
        Rect::new(
            pointer.x - CURSOR_HOTSPOT_X,
            pointer.y - CURSOR_HOTSPOT_Y,
            CURSOR_WIDTH,
            CURSOR_HEIGHT,
        )
    }
}

impl<C> View for CursorLayer<C>
where
    C: View,
{
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);

        let Some(image) = Self::cursor_image() else {
            return;
        };

        let Some(pointer) = self.position_for_paint(bounds) else {
            return;
        };

        let cursor_bounds = Self::cursor_bounds(pointer);

        Image::new(image)
            .content_mode(ImageContentMode::Stretch)
            .sampling(ImageSampling::Bicubic)
            .paint(cursor_bounds, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        match event {
            ViewEvent::PointerMoved { position } => {
                let previous = self.pointer.get();
                if previous != Some(*position) {
                    self.pointer.set(Some(*position));
                    let dirty = previous
                        .map(Self::cursor_bounds)
                        .unwrap_or_else(|| Self::cursor_bounds(*position))
                        .union(Self::cursor_bounds(*position))
                        .expanded(2.0);
                    context.request_redraw_in(dirty);
                }
            }

            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                self.initialized.set(true);
                if let Some(previous) = self.pointer.get() {
                    self.pointer.set(None);
                    context.request_redraw_in(Self::cursor_bounds(previous).expanded(2.0));
                }
            }

            _ => {}
        }

        self.content.handle_event(bounds, event, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewkit::event::RedrawRequest;
    use viewkit::typography::TextMeasurer;

    #[test]
    fn first_motion_erases_initial_cursor_and_focus_loss_does_not_resurrect_it() {
        let pointer = Rc::new(Cell::new(None));
        let initialized = Rc::new(Cell::new(false));
        let layer = CursorLayer::new(Rectangle::new(), pointer.clone(), initialized.clone());
        let bounds = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let initial = Point::new(640.0, 400.0);
        assert_eq!(layer.position_for_paint(bounds), Some(initial));
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        let next = Point::new(100.0, 100.0);
        layer.handle_event(
            bounds,
            &ViewEvent::PointerMoved { position: next },
            &mut context,
        );
        let RedrawRequest::Region(dirty) = context.redraw_request() else {
            panic!("cursor movement must damage both positions");
        };
        assert!(dirty.contains(initial) && dirty.contains(next));
        layer.handle_event(
            bounds,
            &ViewEvent::FocusChanged { focused: false },
            &mut context,
        );
        let rebuilt = CursorLayer::new(Rectangle::new(), pointer, initialized);
        assert_eq!(rebuilt.position_for_paint(bounds), None);
    }
}
