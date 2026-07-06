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
    pub fn open_about(&mut self) -> WindowId {
        if let Some(id) = self
            .windows
            .iter()
            .find(|window| window.content == WindowContent::About)
            .map(|window| window.id)
        {
            self.focus(id);
            return id;
        }

        let id = self.allocate_id();

        self.windows.push(DesktopWindow {
            id,

            title: String::from("About mochiOS"),

            frame: Rect::new(430.0, 190.0, 420.0, 300.0),

            resizable: false,
            minimized: false,

            content: WindowContent::About,
        });

        self.focused = Some(id);

        id
    }

    pub fn focus(&mut self, id: WindowId) {
        let Some(index) = self.windows.iter().position(|window| window.id == id) else {
            return;
        };

        let window = self.windows.remove(index);

        self.windows.push(window);
        self.focused = Some(id);
    }

    pub fn close(&mut self, id: WindowId) {
        self.windows.retain(|window| window.id != id);

        if self.focused == Some(id) {
            self.focused = self.windows.last().map(|window| window.id);
        }
    }

    fn allocate_id(&mut self) -> WindowId {
        let id = WindowId(self.next_id);

        self.next_id = self.next_id.saturating_add(1);

        id
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowDrag {
    pub window: WindowId,

    pub pointer_origin: Point,
    pub window_origin: Point,
}
