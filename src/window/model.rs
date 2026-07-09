use crate::platform::{
    CloseWindowRequest, ProcessId, RemoteWindowId, WindowFocusChangedNotification,
    WindowResizedNotification,
};
use std::collections::{HashMap, HashSet};

use viewkit::prelude::{Point, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowContent {
    About,
    Test,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopWindow {
    pub id: WindowId,
    pub title: String,
    pub frame: Rect,

    pub resizable: bool,
    pub minimized: bool,
    pub close_requested: bool,

    pub content: WindowContent,

    pub process_id: Option<ProcessId>,

    pub remote_window: Option<RemoteWindowId>,

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
    pending_close_requests: Vec<CloseWindowRequest>,
    reported_sizes: HashMap<(ProcessId, RemoteWindowId), (u32, u32)>,
    reported_focus: HashMap<(ProcessId, RemoteWindowId), bool>,
}

impl Default for DesktopWindows {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            focused: None,
            next_id: 1,
            pending_close_requests: Vec::new(),
            reported_sizes: HashMap::new(),
            reported_focus: HashMap::new(),
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

    pub fn open_about(
        &mut self,
        process_id: ProcessId,
        title: String,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> (WindowId, RemoteWindowId) {
        if let Some((id, remote_window)) = self
            .windows
            .iter()
            .find(|window| window.content == WindowContent::About)
            .map(|window| {
                (
                    window.id,
                    window.remote_window.unwrap_or(RemoteWindowId(window.id.0)),
                )
            })
        {
            if let Some(window) = self.windows.iter_mut().find(|window| window.id == id) {
                window.minimized = false;
            }

            self.focus(id);

            return (id, remote_window);
        }

        let id = self.allocate_id();

        let remote_window = RemoteWindowId(id.0);

        self.windows.push(DesktopWindow {
            id,
            title,

            frame: Rect::new(430.0, 190.0, width as f32, height as f32),

            resizable,
            minimized: false,
            close_requested: false,

            content: WindowContent::About,

            process_id: Some(process_id),

            remote_window: Some(remote_window),

            interaction: WindowInteraction::default(),

            restore_frame: None,
        });

        self.focused = Some(id);

        (id, remote_window)
    }

    pub fn process_ids(&self) -> Vec<ProcessId> {
        self.windows
            .iter()
            .filter_map(|window| window.process_id)
            .collect()
    }

    pub fn take_pending_close_requests(&mut self) -> Vec<CloseWindowRequest> {
        std::mem::take(&mut self.pending_close_requests)
    }

    pub fn cancel_close_request(&mut self, process_id: ProcessId, remote_window: RemoteWindowId) {
        if let Some(window) = self.windows.iter_mut().find(|window| {
            window.process_id == Some(process_id) && window.remote_window == Some(remote_window)
        }) {
            window.close_requested = false;
        }
    }

    pub fn close_remote(&mut self, process_id: ProcessId, remote_window: RemoteWindowId) {
        let window_id = self
            .windows
            .iter()
            .find(|window| {
                window.process_id == Some(process_id) && window.remote_window == Some(remote_window)
            })
            .map(|window| window.id);

        if let Some(window_id) = window_id {
            self.remove_window(window_id);
        }
    }

    pub fn close_process(&mut self, process_id: ProcessId) {
        let focused_was_removed = self.focused.is_some_and(|focused| {
            self.windows
                .iter()
                .any(|window| window.id == focused && window.process_id == Some(process_id))
        });

        self.windows
            .retain(|window| window.process_id != Some(process_id));

        self.pending_close_requests
            .retain(|request| request.process_id != process_id);

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

    pub fn close(&mut self, id: WindowId) {
        let mut is_remote = false;

        let mut request = None;

        if let Some(window) = self.windows.iter_mut().find(|window| window.id == id) {
            if let (Some(process_id), Some(remote_window)) =
                (window.process_id, window.remote_window)
            {
                is_remote = true;

                if !window.close_requested {
                    window.close_requested = true;

                    window.interaction = WindowInteraction::default();

                    request = Some(CloseWindowRequest {
                        process_id,
                        window: remote_window,
                    });
                }
            }
        }

        if is_remote {
            if let Some(request) = request {
                self.pending_close_requests.push(request);
            }

            return;
        }

        self.remove_window(id);
    }

    pub fn take_window_state_notifications(
        &mut self,
    ) -> (
        Vec<WindowResizedNotification>,
        Vec<WindowFocusChangedNotification>,
    ) {
        let snapshots: Vec<(ProcessId, RemoteWindowId, u32, u32, bool)> = self
            .windows
            .iter()
            .filter_map(|window| {
                let process_id = window.process_id?;

                let remote_window = window.remote_window?;

                let width = window.frame.size.width.round().clamp(1.0, 16_384.0) as u32;

                let height = window.frame.size.height.round().clamp(1.0, 16_384.0) as u32;

                let focused = self.focused == Some(window.id);

                Some((process_id, remote_window, width, height, focused))
            })
            .collect();

        let active_windows: HashSet<(ProcessId, RemoteWindowId)> = snapshots
            .iter()
            .map(|(process_id, remote_window, _, _, _)| (*process_id, *remote_window))
            .collect();

        let mut resized = Vec::new();

        let mut focus_changed = Vec::new();

        for (process_id, remote_window, width, height, focused) in snapshots {
            let key = (process_id, remote_window);

            let previous_size = self.reported_sizes.insert(key, (width, height));

            if previous_size != Some((width, height)) {
                resized.push(WindowResizedNotification {
                    process_id,
                    window: remote_window,
                    width,
                    height,
                });
            }

            let previous_focus = self.reported_focus.insert(key, focused);

            if previous_focus != Some(focused) {
                focus_changed.push(WindowFocusChangedNotification {
                    process_id,
                    window: remote_window,
                    focused,
                });
            }
        }

        self.reported_sizes
            .retain(|key, _| active_windows.contains(key));

        self.reported_focus
            .retain(|key, _| active_windows.contains(key));

        (resized, focus_changed)
    }

    fn remove_window(&mut self, id: WindowId) {
        let was_focused = self.focused == Some(id);

        self.windows.retain(|window| window.id != id);

        if was_focused {
            self.focus_topmost_visible();
        }
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

    pub fn has_pending_platform_notifications(&self) -> bool {
        if !self.pending_close_requests.is_empty() {
            return true;
        }

        let mut active_windows = HashSet::new();

        for window in &self.windows {
            let (Some(process_id), Some(remote_window)) = (window.process_id, window.remote_window)
            else {
                continue;
            };

            let key = (process_id, remote_window);

            active_windows.insert(key);

            let width = window.frame.size.width.round().clamp(1.0, 16_384.0) as u32;

            let height = window.frame.size.height.round().clamp(1.0, 16_384.0) as u32;

            let size = (width, height);

            if self.reported_sizes.get(&key).copied() != Some(size) {
                return true;
            }

            let focused = self.focused == Some(window.id);

            if self.reported_focus.get(&key).copied() != Some(focused) {
                return true;
            }
        }

        if self
            .reported_sizes
            .keys()
            .any(|key| !active_windows.contains(key))
        {
            return true;
        }

        self.reported_focus
            .keys()
            .any(|key| !active_windows.contains(key))
    }

    fn focus_topmost_visible(&mut self) {
        self.focused = self
            .windows
            .iter()
            .rev()
            .find(|window| !window.minimized)
            .map(|window| window.id);
    }

    pub fn activate_process(&mut self, process_id: ProcessId) {
        let Some(target) = self.activation_target_for_process(process_id) else {
            return;
        };

        self.focus(target);
    }

    fn activation_target_for_process(&self, process_id: ProcessId) -> Option<WindowId> {
        let focused_window = self
            .focused
            .and_then(|focused| self.windows.iter().find(|window| window.id == focused));

        let focused_process_visible = focused_window
            .is_some_and(|window| window.process_id == Some(process_id) && !window.minimized);

        if focused_process_visible {
            let visible_windows: Vec<WindowId> = self
                .windows
                .iter()
                .filter(|window| window.process_id == Some(process_id) && !window.minimized)
                .map(|window| window.id)
                .collect();

            if visible_windows.len() > 1 {
                return visible_windows.first().copied();
            }

            return visible_windows.first().copied();
        }

        self.windows
            .iter()
            .rev()
            .find(|window| window.process_id == Some(process_id) && !window.minimized)
            .or_else(|| {
                self.windows
                    .iter()
                    .rev()
                    .find(|window| window.process_id == Some(process_id))
            })
            .map(|window| window.id)
    }

    pub fn open_test(
        &mut self,
        process_id: ProcessId,
        title: String,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> (WindowId, RemoteWindowId) {
        let id = self.allocate_id();
        let remote_window = RemoteWindowId(id.0);
        let offset = ((id.0.saturating_sub(1) % 6) as f32) * 28.0;

        self.windows.push(DesktopWindow {
            id,
            title,
            frame: Rect::new(360.0 + offset, 160.0 + offset, width as f32, height as f32),
            resizable,
            minimized: false,
            close_requested: false,
            content: WindowContent::Test,
            process_id: Some(process_id),
            remote_window: Some(remote_window),
            interaction: WindowInteraction::default(),
            restore_frame: None,
        });

        self.focused = Some(id);
        (id, remote_window)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowDrag {
    pub window: WindowId,
    pub pointer_origin: Point,
    pub window_origin: Point,
}
