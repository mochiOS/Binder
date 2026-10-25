//! A real child application's display list, placed in a desktop window.

use crate::platform::linux::preview::{Surface, surface};
use std::cell::RefCell;
use std::rc::Rc;
use viewkit::draw_command::DrawCommand;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::platform::linux::preview::Packet;
use viewkit::platform::{ButtonState, PlatformEvent};
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

pub(crate) struct HostSurface(Rc<RefCell<Surface>>);

impl HostSurface {
    pub fn for_renderer(renderer: &str) -> Option<Self> {
        surface(renderer).map(Self)
    }
}

fn translated(mut rect: Rect, origin: Point) -> Rect {
    rect.origin.x += origin.x;
    rect.origin.y += origin.y;
    rect
}

fn place(command: &DrawCommand, bounds: Rect) -> DrawCommand {
    let mut command = command.clone();
    match &mut command {
        DrawCommand::DrawContour { points, .. } | DrawCommand::FillPolygon { points, .. } => {
            for point in points {
                point.x += bounds.origin.x;
                point.y += bounds.origin.y;
            }
        }
        DrawCommand::Clear { color } => {
            return DrawCommand::FillRect {
                rect: bounds,
                color: *color,
            };
        }
        DrawCommand::FillRect { rect, .. }
        | DrawCommand::FillRoundedRect { rect, .. }
        | DrawCommand::FillEllipse { rect, .. }
        | DrawCommand::StrokeEllipse { rect, .. }
        | DrawCommand::StrokeRect { rect, .. }
        | DrawCommand::StrokeRoundedRect { rect, .. }
        | DrawCommand::StrokeInsideRoundedRect { rect, .. }
        | DrawCommand::FillRoundedCornersRect { rect, .. }
        | DrawCommand::StrokeInsideRoundedCornersRect { rect, .. }
        | DrawCommand::PushRoundedCornersClip { rect, .. }
        | DrawCommand::PushClip { rect }
        | DrawCommand::PushRoundedClip { rect, .. } => {
            *rect = translated(*rect, bounds.origin);
        }
        DrawCommand::DrawText { command } => {
            command.bounds = translated(command.bounds, bounds.origin)
        }
        DrawCommand::DrawImage { command } => {
            command.bounds = translated(command.bounds, bounds.origin)
        }
        DrawCommand::DrawSvg { command } => {
            command.bounds = translated(command.bounds, bounds.origin)
        }
        DrawCommand::PopClip => {}
    }
    command
}

impl View for HostSurface {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.maximum
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        context
            .display_list
            .push(DrawCommand::PushClip { rect: bounds });
        for command in self.0.borrow().list.commands() {
            context.display_list.push(place(command, bounds));
        }
        context.display_list.push(DrawCommand::PopClip);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let surface = self.0.borrow();
        if let Some(position) = event.position() {
            if !bounds.contains(position) && !matches!(event, ViewEvent::PointerReleased { .. }) {
                if matches!(event, ViewEvent::PointerMoved { .. }) {
                    let _ = surface.connection.borrow_mut().send(Packet::Event {
                        id: surface.local_id,
                        event: PlatformEvent::PointerLeft,
                    });
                }
                return EventResult::Ignored;
            }
        }
        let local_position = event
            .position()
            .map(|position| Point::new(position.x - bounds.origin.x, position.y - bounds.origin.y));
        let event = match event {
            ViewEvent::PointerMoved { .. } => {
                context.set_cursor(surface.cursor);
                let point = local_position.unwrap();
                PlatformEvent::PointerMoved {
                    x: point.x,
                    y: point.y,
                }
            }
            ViewEvent::PointerPressed { button, .. }
            | ViewEvent::PointerReleased { button, .. } => {
                let point = local_position.unwrap();
                // Button events carry no coordinates in PlatformEvent.
                let _ = surface.connection.borrow_mut().send(Packet::Event {
                    id: surface.local_id,
                    event: PlatformEvent::PointerMoved {
                        x: point.x,
                        y: point.y,
                    },
                });
                PlatformEvent::PointerButton {
                    button: *button,
                    state: if matches!(event, ViewEvent::PointerPressed { .. }) {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                }
            }
            ViewEvent::Scroll {
                delta_x, delta_y, ..
            } => PlatformEvent::Scroll {
                delta_x: *delta_x,
                delta_y: *delta_y,
            },
            ViewEvent::PointerLeft => PlatformEvent::PointerLeft,
            ViewEvent::KeyPressed { key, modifiers } => PlatformEvent::KeyPressed {
                key: *key,
                modifiers: *modifiers,
            },
            ViewEvent::KeyReleased { key, modifiers } => PlatformEvent::KeyReleased {
                key: *key,
                modifiers: *modifiers,
            },
            ViewEvent::TextInput { text } => PlatformEvent::TextInput { text: text.clone() },
            ViewEvent::FocusChanged { focused } => PlatformEvent::Focused(*focused),
            ViewEvent::Backspace => PlatformEvent::Backspace,
            ViewEvent::Delete => PlatformEvent::Delete,
            ViewEvent::ArrowLeft => PlatformEvent::ArrowLeft,
            ViewEvent::ArrowRight => PlatformEvent::ArrowRight,
            ViewEvent::Home => PlatformEvent::Home,
            ViewEvent::End => PlatformEvent::End,
            ViewEvent::SelectLeft => PlatformEvent::SelectLeft,
            ViewEvent::SelectRight => PlatformEvent::SelectRight,
            ViewEvent::SelectHome => PlatformEvent::SelectHome,
            ViewEvent::SelectEnd => PlatformEvent::SelectEnd,
            ViewEvent::SelectAll => PlatformEvent::SelectAll,
            ViewEvent::ContextMenuResult {
                request_id,
                command_id,
            } => PlatformEvent::ContextMenuResult {
                request_id: *request_id,
                command_id: *command_id,
            },
            ViewEvent::FileDragEntered { path, .. } => {
                PlatformEvent::FileHovered { path: path.clone() }
            }
            ViewEvent::FileDragExited => PlatformEvent::FileHoverCancelled,
            ViewEvent::FileDropped { path, .. } => {
                PlatformEvent::FileDropped { path: path.clone() }
            }
            _ => return EventResult::Ignored,
        };
        if let Err(error) = surface.connection.borrow_mut().send(Packet::Event {
            id: surface.local_id,
            event,
        }) {
            eprintln!("failed to deliver preview input: {error}");
        }
        context.request_redraw();
        EventResult::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_clear_cannot_clear_the_desktop_and_coordinates_are_local() {
        let bounds = Rect::new(100.0, 200.0, 300.0, 400.0);
        assert_eq!(
            place(
                &DrawCommand::Clear {
                    color: Color::WHITE
                },
                bounds
            ),
            DrawCommand::FillRect {
                rect: bounds,
                color: Color::WHITE
            }
        );
        assert_eq!(
            place(
                &DrawCommand::PushClip {
                    rect: Rect::new(2.0, 3.0, 4.0, 5.0)
                },
                bounds
            ),
            DrawCommand::PushClip {
                rect: Rect::new(102.0, 203.0, 4.0, 5.0)
            }
        );
    }
    #[test]
    fn places_native_contour_points_inside_the_child_window() {
        let command = DrawCommand::DrawContour {
            points: vec![
                Point::new(0.0, 0.0),
                Point::new(30.0, 0.0),
                Point::new(15.0, 10.0),
            ],
            fill: Color::WHITE,
            stroke: Color::BLACK,
            stroke_width: 1.0,
        };
        let DrawCommand::DrawContour {
            points,
            stroke_width,
            ..
        } = place(&command, Rect::new(100.0, 200.0, 300.0, 400.0))
        else {
            panic!()
        };
        assert_eq!(
            points,
            vec![
                Point::new(100.0, 200.0),
                Point::new(130.0, 200.0),
                Point::new(115.0, 210.0)
            ]
        );
        assert_eq!(stroke_width, 1.0);
    }
    #[test]
    fn places_omochi_polygons_inside_the_child_window() {
        let command = DrawCommand::FillPolygon {
            points: vec![Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(15.0, 10.0)],
            color: Color::WHITE,
        };
        assert_eq!(place(&command, Rect::new(100.0, 200.0, 300.0, 400.0)),
            DrawCommand::FillPolygon {
                points: vec![Point::new(100.0, 200.0), Point::new(130.0, 200.0), Point::new(115.0, 210.0)],
                color: Color::WHITE,
            });
    }

}
