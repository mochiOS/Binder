use std::sync::OnceLock;

use viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const CURSOR_SVG_PATH: &str = "/system/icons/cursor.svg";
const CURSOR_SVG_BYTES: &[u8] = include_bytes!(concat!(
env!("CARGO_MANIFEST_DIR"),
"/../../resources/system/icons/cursor.svg"
));
const CURSOR_WIDTH: f32 = 35.0;
const CURSOR_HEIGHT: f32 = 60.0;
const CURSOR_HOTSPOT_X: f32 = 1.0;
const CURSOR_HOTSPOT_Y: f32 = 1.0;

pub(crate) struct CursorLayer<C> {
    content: C,
    pointer: State<Option<Point>>,
}

impl<C> CursorLayer<C>
where
    C: View,
{
    pub(crate) fn new(content: C, pointer: State<Option<Point>>) -> Self {
        Self { content, pointer }
    }

    fn cursor_svg() -> Option<SvgData> {
        static CURSOR: OnceLock<Option<SvgData>> = OnceLock::new();

        CURSOR
            .get_or_init(|| {
                SvgData::from_path(CURSOR_SVG_PATH)
                    .or_else(|_| SvgData::decode(CURSOR_SVG_BYTES))
                    .ok()
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

        let Some(svg) = Self::cursor_svg() else {
            return;
        };

        let pointer = self.pointer.get().unwrap_or_else(|| {
            Point::new(
                bounds.origin.x + bounds.size.width / 2.0,
                bounds.origin.y + bounds.size.height / 2.0,
            )
        });

        let cursor_bounds = Self::cursor_bounds(pointer);

        Svg::new(svg)
            .content_mode(SvgContentMode::Stretch)
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
