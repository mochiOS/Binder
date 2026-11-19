//! One recognizer owns each contact from direction lock to release/cancel.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use viewkit::geometry::Point;

pub(super) const SLOP: f32 = 10.0;
const LONG_PRESS: Duration = Duration::from_millis(450);
const SWITCHER_HOLD: Duration = Duration::from_millis(260);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Home(Option<u64>),
    HomeLibrary,
    HomeNotifications,
    TopControls,
    TopNotifications,
    Edge,
    Library,
    LibraryList,
    Folder,
    Notifications,
    NotificationList,
    Switcher,
    Dock,
    Tap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Pending,
    Horizontal,
    Vertical,
    Drag(u64),
    Held,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    Tap(Point),
    EdgeTap(Point),
    Pan {
        delta: f32,
        velocity: f32,
        finished: bool,
    },
    Sheet {
        origin: Origin,
        delta: f32,
        velocity: f32,
        finished: bool,
    },
    FinishNavigation {
        delta: f32,
        velocity: f32,
        delta_x: f32,
        velocity_x: f32,
        home: bool,
    },
    RevealNavigation,
    Navigate {
        delta: f32,
        velocity: f32,
        delta_x: f32,
        velocity_x: f32,
    },
    Switcher,
    QuickSwitch(i32),
    Edit(u64, Point),
    Drag(u64, Point),
    Drop(u64, Point),
    Cancel,
}

#[derive(Clone, Copy)]
struct Contact {
    origin: Origin,
    start: Point,
    last: Point,
    started: Instant,
    moved: Instant,
    hold_anchor: Point,
    held_since: Instant,
    velocity: Point,
    kind: Kind,
    editing: bool,
    passed_slop: bool,
}

#[derive(Default)]
pub(super) struct Gestures {
    contacts: HashMap<u64, Contact>,
    owner: Option<u64>,
}

impl Gestures {
    pub fn begin(
        &mut self,
        id: u64,
        origin: Origin,
        point: Point,
        editing: bool,
        now: Instant,
    ) -> bool {
        if self.contacts.len() >= 16 || self.contacts.contains_key(&id) {
            return false;
        }
        self.contacts.insert(
            id,
            Contact {
                origin,
                start: point,
                last: point,
                started: now,
                moved: now,
                hold_anchor: point,
                held_since: now,
                velocity: Point::new(0.0, 0.0),
                kind: Kind::Pending,
                editing,
                passed_slop: false,
            },
        );
        if self.owner.is_none() {
            self.owner = Some(id);
            return true;
        }
        false
    }

    pub fn contains(&self, id: u64) -> bool {
        self.contacts.contains_key(&id)
    }
    pub fn owns(&self, id: u64) -> bool {
        self.owner == Some(id)
    }
    pub fn active(&self) -> bool {
        self.owner.is_some()
    }
    pub fn edge_active(&self) -> bool {
        self.owner
            .and_then(|id| self.contacts.get(&id))
            .is_some_and(|contact| contact.origin == Origin::Edge)
    }

    pub fn update(&mut self, id: u64, point: Point, now: Instant) -> Option<Action> {
        if self.owner != Some(id) {
            return None;
        }
        let contact = self.contacts.get_mut(&id)?;
        let dt = now.saturating_duration_since(contact.moved).as_secs_f32();
        let moved = (point.x - contact.last.x).abs() + (point.y - contact.last.y).abs() > 1.0;
        if moved && dt > 0.0001 {
            contact.velocity = Point::new(
                (point.x - contact.last.x) / dt,
                (point.y - contact.last.y) / dt,
            );
        }
        if moved {
            contact.moved = now;
        } else if dt > 0.08 {
            contact.velocity = Point::new(0.0, 0.0);
        }
        contact.last = point;
        if (point.x - contact.hold_anchor.x).hypot(point.y - contact.hold_anchor.y) >= 6.0 {
            contact.hold_anchor = point;
            contact.held_since = now;
        }
        let dx = point.x - contact.start.x;
        let dy = point.y - contact.start.y;
        let moved_outside_slop = dx.hypot(dy) >= SLOP;
        contact.passed_slop |= moved_outside_slop;
        if contact.kind == Kind::Pending && moved_outside_slop {
            contact.kind = match contact.origin {
                Origin::Home(Some(id)) if contact.editing => Kind::Drag(id),
                Origin::Tap => Kind::Held,
                _ if dx.abs() > dy.abs() * 1.15 => Kind::Horizontal,
                _ if dy.abs() > dx.abs() * 1.15 => Kind::Vertical,
                _ => Kind::Pending,
            };
            if contact.kind == Kind::Vertical && matches!(contact.origin, Origin::Home(_)) {
                contact.origin = if dy < 0.0 {
                    Origin::HomeLibrary
                } else {
                    Origin::HomeNotifications
                };
            }
            if contact.kind == Kind::Vertical && contact.origin == Origin::Library && dy < 0.0 {
                contact.origin = Origin::LibraryList;
            }
        }
        match contact.kind {
            Kind::Horizontal if contact.origin == Origin::Edge => Some(Action::RevealNavigation),
            Kind::Vertical if contact.origin == Origin::Edge => Some(Action::Navigate {
                delta: dy,
                velocity: contact.velocity.y,
                delta_x: dx,
                velocity_x: contact.velocity.x,
            }),
            Kind::Horizontal if matches!(contact.origin, Origin::Home(_) | Origin::Switcher) => {
                Some(Action::Pan {
                    delta: -dx,
                    velocity: -contact.velocity.x,
                    finished: false,
                })
            }
            Kind::Vertical if contact.origin != Origin::Edge => Some(Action::Sheet {
                origin: contact.origin,
                delta: dy,
                velocity: contact.velocity.y,
                finished: false,
            }),
            Kind::Drag(id) => Some(Action::Drag(id, point)),
            _ => None,
        }
    }

    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        let contact = self.contacts.get_mut(&self.owner?)?;
        if contact.kind == Kind::Pending
            && !contact.passed_slop
            && now.saturating_duration_since(contact.started) >= LONG_PRESS
        {
            if let Origin::Home(Some(id)) = contact.origin {
                contact.kind = Kind::Drag(id);
                contact.editing = true;
                return Some(Action::Edit(id, contact.last));
            }
        }
        if contact.origin == Origin::Edge
            && contact.kind == Kind::Vertical
            && contact.last.y < contact.start.y - 48.0
            && now.saturating_duration_since(contact.held_since) >= SWITCHER_HOLD
        {
            contact.kind = Kind::Held;
            return Some(Action::Switcher);
        }
        None
    }

    pub fn deadline(&self) -> Option<Instant> {
        let contact = self.contacts.get(&self.owner?)?;
        match contact.kind {
            Kind::Pending if !contact.passed_slop => Some(contact.started + LONG_PRESS),
            Kind::Vertical if contact.origin == Origin::Edge => {
                Some(contact.held_since + SWITCHER_HOLD)
            }
            _ => None,
        }
    }

    pub fn end(&mut self, id: u64, point: Point, now: Instant) -> Option<Action> {
        let action = self.update(id, point, now);
        let contact = self.contacts.remove(&id)?;
        if self.owner != Some(id) {
            return None;
        }
        self.owner = None;
        let dx = point.x - contact.start.x;
        let dy = point.y - contact.start.y;
        // Input timing, rather than paint cadence, decides a completed hold.
        if contact.origin == Origin::Edge
            && contact.kind == Kind::Vertical
            && dy < -48.0
            && now.saturating_duration_since(contact.held_since) >= SWITCHER_HOLD
        {
            return Some(Action::Switcher);
        }
        match contact.kind {
            Kind::Pending if contact.passed_slop => Some(Action::Cancel),
            Kind::Pending if contact.origin == Origin::Edge => Some(Action::EdgeTap(point)),
            Kind::Pending => Some(Action::Tap(point)),
            Kind::Drag(item) => Some(Action::Drop(item, point)),
            Kind::Horizontal if contact.origin == Origin::Edge && dx.abs() >= 36.0 => {
                Some(Action::QuickSwitch(if dx < 0.0 { 1 } else { -1 }))
            }
            Kind::Vertical if contact.origin == Origin::Edge => {
                // Respect a reversal and a short deliberate upward flick. A
                // distance threshold alone closes apps even while pulling back.
                let projected = dy + contact.velocity.y * 0.18;
                let home = dy < -SLOP && projected < -72.0 && contact.velocity.y < 120.0;
                Some(Action::FinishNavigation {
                    delta: dy,
                    velocity: contact.velocity.y,
                    delta_x: dx,
                    velocity_x: contact.velocity.x,
                    home,
                })
            }
            _ => match action {
                Some(Action::Pan {
                    delta, velocity, ..
                }) => Some(Action::Pan {
                    delta,
                    velocity,
                    finished: true,
                }),
                Some(Action::Sheet {
                    origin,
                    delta,
                    velocity,
                    ..
                }) => Some(Action::Sheet {
                    origin,
                    delta,
                    velocity,
                    finished: true,
                }),
                _ => Some(Action::Cancel),
            },
        }
    }

    pub fn cancel(&mut self, id: u64) -> Option<Action> {
        self.contacts.remove(&id);
        if self.owner == Some(id) {
            self.owner = None;
            Some(Action::Cancel)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.contacts.clear();
        self.owner = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undecided_drag_cannot_turn_into_a_tap_or_long_press() {
        let now = Instant::now();
        let start = Point::new(100.0, 100.0);
        let diagonal = Point::new(190.0, 190.0);
        let mut gestures = Gestures::default();
        assert!(gestures.begin(7, Origin::Home(Some(1)), start, false, now));
        assert_eq!(
            gestures.update(7, diagonal, now + Duration::from_millis(100)),
            None
        );
        assert_eq!(gestures.tick(now + LONG_PRESS), None);
        assert_eq!(
            gestures.end(7, start, now + LONG_PRESS),
            Some(Action::Cancel)
        );
    }
    #[test]
    fn upward_flick_uses_the_release_position_and_velocity() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Edge, Point::new(200., 790.), false, now);
        gestures.update(1, Point::new(200., 775.), now + Duration::from_millis(20));
        let release = gestures.end(1, Point::new(220., 750.), now + Duration::from_millis(40));
        assert!(matches!(release, Some(Action::FinishNavigation {
            delta: -40.0, delta_x: 20.0, velocity, velocity_x, home: true,
        }) if velocity < -1000.0 && velocity_x > 900.0));
    }

    #[test]
    fn reversing_an_upward_drag_restores_the_application() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Edge, Point::new(200., 790.), false, now);
        gestures.update(1, Point::new(200., 550.), now + Duration::from_millis(90));
        let release = gestures.end(1, Point::new(200., 680.), now + Duration::from_millis(160));
        assert!(
            matches!(release, Some(Action::FinishNavigation { home: false, velocity, .. }) if velocity > 0.0)
        );
    }

    #[test]
    fn short_slow_pull_does_not_leave_the_application() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Edge, Point::new(200., 790.), false, now);
        gestures.update(1, Point::new(200., 770.), now + Duration::from_millis(50));
        let release = gestures.end(1, Point::new(200., 770.), now + Duration::from_millis(180));
        assert!(matches!(
            release,
            Some(Action::FinishNavigation {
                home: false,
                velocity: 0.0,
                ..
            })
        ));
    }

    #[test]
    fn upper_corner_drag_retains_control_center_ownership_across_the_screen() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(8, Origin::TopControls, Point::new(1200.0, 20.0), false, now);
        assert!(matches!(
            gestures.update(
                8,
                Point::new(1190.0, 300.0),
                now + Duration::from_millis(100)
            ),
            Some(Action::Sheet {
                origin: Origin::TopControls,
                ..
            })
        ));
        assert!(matches!(
            gestures.end(
                8,
                Point::new(600.0, 350.0),
                now + Duration::from_millis(200)
            ),
            Some(Action::Sheet {
                origin: Origin::TopControls,
                finished: true,
                ..
            })
        ));
    }
    #[test]
    fn edge_hold_survives_small_jitter_without_a_paint_tick() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Edge, Point::new(200.0, 790.0), false, now);
        gestures.update(
            1,
            Point::new(200.0, 500.0),
            now + Duration::from_millis(100),
        );
        gestures.update(
            1,
            Point::new(202.0, 501.0),
            now + Duration::from_millis(250),
        );
        assert_eq!(
            gestures.end(
                1,
                Point::new(201.0, 500.0),
                now + Duration::from_millis(400)
            ),
            Some(Action::Switcher)
        );
    }
    #[test]
    fn direction_locks_and_secondary_contacts_never_activate_another_action() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Home(None), Point::new(200., 200.), false, now);
        gestures.begin(2, Origin::Edge, Point::new(200., 790.), false, now);
        assert!(gestures.update(1, Point::new(204., 203.), now).is_none());
        assert!(matches!(
            gestures.update(1, Point::new(100., 205.), now + Duration::from_millis(50)),
            Some(Action::Pan { .. })
        ));
        assert_eq!(gestures.end(2, Point::new(200., 500.), now), None);
        assert!(matches!(
            gestures.end(1, Point::new(90., 100.), now + Duration::from_millis(100)),
            Some(Action::Pan { finished: true, .. })
        ));
    }
    #[test]
    fn edge_hold_and_long_press_are_exclusive_and_cancellation_is_not_a_tap() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Edge, Point::new(100., 790.), false, now);
        gestures.update(1, Point::new(100., 650.), now + Duration::from_millis(100));
        assert_eq!(
            gestures.tick(now + Duration::from_millis(400)),
            Some(Action::Switcher)
        );
        assert_eq!(
            gestures.end(1, Point::new(100., 650.), now + Duration::from_millis(410)),
            Some(Action::Cancel)
        );
        gestures.begin(
            4,
            Origin::Home(Some(17)),
            Point::new(100., 100.),
            false,
            now,
        );
        assert_eq!(
            gestures.tick(now + LONG_PRESS),
            Some(Action::Edit(17, Point::new(100., 100.)))
        );
        assert_eq!(gestures.cancel(4), Some(Action::Cancel));
        assert_eq!(gestures.end(4, Point::new(100., 100.), now), None);
    }

    #[test]
    fn reversing_direction_cannot_change_the_claimed_sheet_or_turn_list_scrolling_into_dismissal() {
        let now = Instant::now();
        let mut gestures = Gestures::default();
        gestures.begin(1, Origin::Home(None), Point::new(200., 400.), false, now);
        assert!(matches!(
            gestures.update(1, Point::new(200., 200.), now + Duration::from_millis(30)),
            Some(Action::Sheet {
                origin: Origin::HomeLibrary,
                ..
            })
        ));
        assert!(matches!(
            gestures.end(1, Point::new(200., 500.), now + Duration::from_millis(70)),
            Some(Action::Sheet {
                origin: Origin::HomeLibrary,
                finished: true,
                ..
            })
        ));
        gestures.begin(2, Origin::Library, Point::new(200., 400.), false, now);
        assert!(matches!(
            gestures.update(2, Point::new(200., 200.), now + Duration::from_millis(30)),
            Some(Action::Sheet {
                origin: Origin::LibraryList,
                ..
            })
        ));
        assert!(matches!(
            gestures.end(2, Point::new(200., 500.), now + Duration::from_millis(70)),
            Some(Action::Sheet {
                origin: Origin::LibraryList,
                finished: true,
                ..
            })
        ));
    }
}
