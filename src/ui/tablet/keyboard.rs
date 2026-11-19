use std::{fs, io::Read};
use viewkit::prelude::*;
use viewkit::software_keyboard::{
    KeyboardLayout, SoftwareKeyboard, parse_layouts, preferred_layout, save_preferred_layout,
};

use crate::platform::{AppInfo, ProcessId, RemoteWindowId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Search,
    Application(ProcessId, RemoteWindowId),
}

#[derive(Default)]
pub(super) struct Keyboard {
    pub layouts: Vec<KeyboardLayout>,
    pub panel: SoftwareKeyboard,
    pub target: Option<Target>,
    dismissed: Option<Target>,
    ids: Vec<String>,
}

impl Keyboard {
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
        self.panel.cancel();
    }

    pub fn resume(&mut self) {
        self.dismissed = None;
    }

    pub fn bounds(&self, edge: Rect, height: f32, theme: &Theme) -> Option<Rect> {
        if self.target.is_none() || self.dismissed.is_some() || self.layouts.is_empty() {
            return None;
        }
        let height = SoftwareKeyboard::height(&self.layouts, theme).min(height * 0.5);
        Some(Rect::new(
            edge.origin.x,
            edge.origin.y - height,
            edge.size.width,
            height,
        ))
    }
}
