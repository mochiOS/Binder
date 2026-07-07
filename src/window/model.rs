use crate::platform::ProcessId;

use viewkit::prelude::{Point, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowContent {
    About,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopWindow {
    pub id: WindowId,
    pub title: String,
    pub frame: Rect,

    pub resizable: bool,
    pub minimized: bool,

    pub content: WindowContent,

    pub process_id: Option<ProcessId>,

    pub interaction: WindowInteraction,
    pub restore_frame: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowInteraction {
    pub hovered: Option<WindowControl>,
    pub pressed: Option<WindowControl>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopWindows {
    pub windows: Vec<DesktopWindow>,
    pub focused: Option<WindowId>,

    next_id: u64,
}

impl Default for DesktopWindows {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            focused: None,
            next_id: 1,
        }
    }
}

impl DesktopWindows {
    pub fn about_window(&self) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|window| window.content == WindowContent::About)
            .map(|window| window.id)
    }

    pub fn open_about(&mut self, process_id: ProcessId) -> WindowId {
        let id = self.allocate_id();

        self.windows.push(DesktopWindow {
            id,

            title: String::from("About mochiOS"),

            frame: Rect::new(430.0, 190.0, 420.0, 300.0),

            resizable: true,
            minimized: false,

            content: WindowContent::About,

            process_id: Some(process_id),

            interaction: WindowInteraction::default(),

            restore_frame: None,
        });

        self.focused = Some(id);

        id
    }

    pub fn process_ids(&self) -> Vec<ProcessId> {
        self.windows
            .iter()
            .filter_map(|window| window.process_id)
            .collect()
    }

    pub fn close_process(&mut self, process_id: ProcessId) {
        let focused_was_removed = self
            .focused
            .and_then(|focused| self.windows.iter().find(|window| window.id == focused))
            .and_then(|window| window.process_id)
            == Some(process_id);

        self.windows
            .retain(|window| window.process_id != Some(process_id));

        if focused_was_removed {
            self.focus_topmost_visible();
        }
    }

    pub fn focus(&mut self, id: WindowId) {
        let Some(index) = self.windows.iter().position(|window| window.id == id) else {
            return;
        };

        let mut window = self.windows.remove(index);

        window.minimized = false;

        self.windows.push(window);
        self.focused = Some(id);
    }

    pub fn close(&mut self, id: WindowId) -> Option<ProcessId> {
        let process_id = self
            .windows
            .iter()
            .find(|window| window.id == id)
            .and_then(|window| window.process_id);

        let was_focused = self.focused == Some(id);

        self.windows.retain(|window| window.id != id);

        if was_focused {
            self.focus_topmost_visible();
        }

        process_id
    }

    fn allocate_id(&mut self) -> WindowId {
        let id = WindowId(self.next_id);

        self.next_id = self.next_id.saturating_add(1);

        id
    }

    pub fn set_hovered_control(&mut self, target: Option<(WindowId, WindowControl)>) {
        for window in &mut self.windows {
            window.interaction.hovered = match target {
                Some((id, control)) if id == window.id => Some(control),

                _ => None,
            };
        }
    }

    pub fn clear_pressed_controls(&mut self) {
        for window in &mut self.windows {
            window.interaction.pressed = None;
        }
    }

    pub fn clear_interactions(&mut self) {
        for window in &mut self.windows {
            window.interaction = WindowInteraction::default();
        }
    }

    pub fn press_control(&mut self, id: WindowId, control: WindowControl) {
        self.clear_pressed_controls();

        if let Some(window) = self.windows.iter_mut().find(|window| window.id == id) {
            window.interaction.pressed = Some(control);

            window.interaction.hovered = Some(control);
        }
    }

    pub fn minimize(&mut self, id: WindowId) {
        if let Some(window) = self.windows.iter_mut().find(|window| window.id == id) {
            window.minimized = true;

            window.interaction = WindowInteraction::default();
        }

        if self.focused == Some(id) {
            self.focus_topmost_visible();
        }
    }

    pub fn toggle_maximize(&mut self, id: WindowId, work_area: Rect) {
        let Some(window) = self.windows.iter_mut().find(|window| window.id == id) else {
            return;
        };

        if let Some(restore_frame) = window.restore_frame.take() {
            window.frame = restore_frame;
        } else {
            window.restore_frame = Some(window.frame);

            window.frame = work_area;
        }

        window.interaction = WindowInteraction::default();

        self.focus(id);
    }

    fn focus_topmost_visible(&mut self) {
        self.focused = self
            .windows
            .iter()
            .rev()
            .find(|window| !window.minimized)
            .map(|window| window.id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowDrag {
    pub window: WindowId,

    pub pointer_origin: Point,
    pub window_origin: Point,
}
