mod placement;

use placement::{FloatingFrame, Placement};
use std::{fs, io, io::Read};
use viewkit::accessibility::{AccessibilityNode, AccessibilityRole};
use viewkit::event::{EventContext, ViewEvent};
use viewkit::platform::{CursorIcon, PointerButton, TouchPhase};
use viewkit::prelude::*;
use viewkit::software_keyboard::{
    KeyboardLayout, SoftwareKeyboard, parse_layouts, preferred_layout, save_preferred_layout,
};
use viewkit::view::PaintContext;

use super::MOUSE;
use crate::platform::{AppInfo, ProcessId, RemoteWindowId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Search,
    Application(ProcessId, RemoteWindowId),
}

#[derive(Clone, Copy)]
pub(super) struct KeyboardGeometry {
    pub bounds: Rect,
    pub keys: Rect,
    floating: Option<FloatingFrame>,
}

impl KeyboardGeometry {
    pub fn docked(self) -> bool {
        self.floating.is_none()
    }

    fn handles(self) -> Option<(Rect, Rect)> {
        let header = self.floating?.header;
        let width = header.min(self.bounds.size.width * 0.33);
        Some((
            Rect::new(self.bounds.origin.x, self.bounds.origin.y, width, header),
            Rect::new(
                self.bounds.origin.x + width,
                self.bounds.origin.y,
                self.bounds.size.width - width,
                header,
            ),
        ))
    }
}

struct Drag {
    id: u64,
    start: Point,
    initial: Placement,
    panel: Rect,
    resizing: bool,
}

#[derive(Default)]
pub(super) struct Keyboard {
    pub layouts: Vec<KeyboardLayout>,
    pub panel: SoftwareKeyboard,
    pub target: Option<Target>,
    dismissed: Option<Target>,
    ids: Vec<String>,
    placement: Placement,
    drag: Option<Drag>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            placement: Placement::load(),
            ..Self::default()
        }
    }

    pub fn load(&mut self, apps: &[AppInfo]) {
        let selected = self
            .ids
            .get(self.panel.selected)
            .cloned()
            .or_else(preferred_layout);
        self.layouts.clear();
        self.ids.clear();
        let mut providers = apps.iter().collect::<Vec<_>>();
        providers.sort_by_key(|app| app.bundle_id != "org.mochios.keyboard");
        for app in providers {
            let path = app.root.join("keyboard.layout");
            if !app.resources.contains(&path) {
                continue;
            }
            let Ok(file) = fs::File::open(&path) else {
                continue;
            };
            let mut contents = String::new();
            if file
                .take(16 * 1024 + 1)
                .read_to_string(&mut contents)
                .is_err()
            {
                continue;
            }
            let Some(layouts) = parse_layouts(&contents) else {
                continue;
            };
            for mut layout in layouts {
                let id = format!("{}/{}", app.bundle_id, layout.name);
                if self.ids.contains(&id) {
                    continue;
                }
                if app.bundle_id != "org.mochios.keyboard" {
                    layout.name = format!("{} · {}", app.name, layout.name);
                }
                self.ids.push(id);
                self.layouts.push(layout);
            }
        }
        self.cancel_drag();
        self.panel.cancel();
        self.panel.selected = selected
            .and_then(|id| self.ids.iter().position(|candidate| candidate == &id))
            .unwrap_or(0);
    }

    pub fn save(&self) {
        if let Some(id) = self.ids.get(self.panel.selected) {
            let _ = save_preferred_layout(id);
        }
    }

    pub fn update_target(&mut self, target: Option<Target>) {
        if self.dismissed != target {
            self.dismissed = None;
        }
        if self.target != target {
            self.cancel_drag();
            self.panel.cancel();
            if target.is_some()
                && let Some(id) = preferred_layout()
                && let Some(index) = self.ids.iter().position(|candidate| candidate == &id)
            {
                self.panel.selected = index;
            }
            self.target = target;
        }
    }

    pub fn dismiss(&mut self) {
        self.dismissed = self.target;
        self.cancel_drag();
        self.panel.cancel();
    }

    pub fn resume(&mut self) {
        self.dismissed = None;
    }

    pub fn geometry(
        &self,
        edge: Rect,
        application: Rect,
        theme: &Theme,
    ) -> Option<KeyboardGeometry> {
        if self.target.is_none() || self.dismissed.is_some() {
            return None;
        }
        let layout = self.layouts.get(self.panel.selected)?;
        if !layout.kana {
            let width = layout.preferred_width(edge.size.width, theme);
            let height = layout
                .preferred_height(width, theme)
                .min(application.size.height * 0.54);
            let bounds = Rect::new(
                edge.origin.x,
                edge.origin.y - height,
                edge.size.width,
                height,
            );
            return Some(KeyboardGeometry {
                bounds,
                keys: bounds,
                floating: None,
            });
        }
        let gap = theme.spacing.medium.min(edge.size.width / 12.0);
        let area = Rect::new(
            edge.origin.x + gap,
            application.origin.y + gap,
            edge.size.width - 2.0 * gap,
            edge.origin.y - application.origin.y - 2.0 * gap,
        );
        if area.is_empty() {
            return None;
        }
        let width = layout.preferred_width(area.size.width, theme);
        let frame = FloatingFrame {
            area,
            header: theme.controls.touch_height.min(area.size.height / 3.0),
            base: Size::new(width, layout.preferred_height(width, theme)),
        };
        let panel = frame.bounds(self.placement);
        Some(KeyboardGeometry {
            bounds: panel,
            keys: Rect::new(
                panel.origin.x,
                panel.origin.y + frame.header,
                panel.size.width,
                panel.size.height - frame.header,
            ),
            floating: Some(frame),
        })
    }

    pub fn cancel_drag(&mut self) -> bool {
        if let Some(drag) = self.drag.take() {
            self.placement = drag.initial;
            true
        } else {
            false
        }
    }

    pub fn handle_placement(
        &mut self,
        geometry: KeyboardGeometry,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> io::Result<bool> {
        let Some(frame) = geometry.floating else {
            return Ok(false);
        };
        let (id, phase, point) = match *event {
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => (MOUSE, TouchPhase::Started, position),
            ViewEvent::PointerMoved { position } => (MOUSE, TouchPhase::Moved, position),
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => (MOUSE, TouchPhase::Ended, position),
            ViewEvent::Touch {
                id,
                phase,
                position,
            } => (id, phase, position),
            ViewEvent::PointerLeft if self.drag.as_ref().is_some_and(|drag| drag.id == MOUSE) => {
                return Ok(self.cancel_drag());
            }
            ViewEvent::FocusChanged { focused: false } => return Ok(self.cancel_drag()),
            _ => return Ok(false),
        };
        let (resize, grip) = geometry.handles().unwrap();
        let resizing = resize.contains(point);
        if id == MOUSE && (geometry.bounds.contains(point) || self.drag.is_some()) {
            let cursor = match &self.drag {
                Some(drag) if drag.resizing => CursorIcon::NwseResize,
                Some(_) => CursorIcon::Pointer,
                None if resizing => CursorIcon::NwseResize,
                None if grip.contains(point) => CursorIcon::Pointer,
                None => CursorIcon::Default,
            };
            context.set_cursor(cursor);
        }
        if let Some(drag) = &self.drag {
            if drag.id != id {
                return Ok(geometry.bounds.contains(point));
            }
            if phase == TouchPhase::Cancelled {
                return Ok(self.cancel_drag());
            }
            if phase != TouchPhase::Started {
                let delta = Point::new(point.x - drag.start.x, point.y - drag.start.y);
                self.placement = if delta == Point::default() {
                    drag.initial
                } else {
                    frame.adjust(drag.initial, drag.panel, delta, drag.resizing)
                };
            }
            if phase == TouchPhase::Ended {
                let initial = drag.initial;
                self.drag = None;
                if self.placement != initial {
                    self.placement.save()?;
                }
            }
            return Ok(true);
        }
        if phase == TouchPhase::Started && (resizing || grip.contains(point)) {
            self.panel.cancel();
            self.drag = Some(Drag {
                id,
                start: point,
                initial: self.placement,
                panel: geometry.bounds,
                resizing,
            });
            return Ok(true);
        }
        Ok(false)
    }

    pub fn paint(
        &self,
        geometry: KeyboardGeometry,
        footer: f32,
        japanese: bool,
        context: &mut PaintContext<'_>,
    ) {
        let Some((resize, grip)) = geometry.handles() else {
            self.panel
                .paint_with_footer(&self.layouts, geometry.keys, footer, japanese, context);
            return;
        };
        self.panel.paint_floating(
            &self.layouts,
            geometry.keys,
            geometry.bounds,
            japanese,
            context,
        );
        let theme = context.theme;
        let color = theme.colors.text_secondary;
        Icon::new(SymbolName::ArrowTopLeft)
            .size(theme.layout.control_icon_size)
            .color(color)
            .paint(resize, context);
        let width = theme.controls.touch_height.min(grip.size.width * 0.5);
        let height = theme.spacing.extra_small;
        // Center the grabber on the tray when the resize control leaves room.
        let center = (geometry.bounds.origin.x + geometry.bounds.size.width * 0.5)
            .max(grip.origin.x + width * 0.5);
        Rectangle::new()
            .color(RectangleColor::Custom(color))
            .radius(CornerRadius::Full)
            .paint(
                Rect::new(
                    center - width * 0.5,
                    grip.origin.y + (grip.size.height - height) * 0.5,
                    width,
                    height,
                ),
                context,
            );
        for (bounds, label) in [(resize, "Resize keyboard"), (grip, "Move keyboard")] {
            let mut node = AccessibilityNode::new(AccessibilityRole::Group, bounds);
            node.label = Some(label.into());
            context.record_accessibility(node);
        }
    }
}
