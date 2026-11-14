//! Persistent grid coordinates contain identities, never application metadata.
use serde_json::{Value, json};
use std::{collections::HashSet, fs, io, path::PathBuf};

pub(super) const ROWS: u32 = 4;
const MAX_ITEMS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slot {
    pub column: u32,
    pub row: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ItemKind {
    Application(String),
    Folder {
        name: String,
        applications: Vec<String>,
    },
    /// Reserved placement schema; no fake widget is populated by the shell.
    Widget {
        provider: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HomeItem {
    pub id: u64,
    pub slot: Slot,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct HomeLayout {
    pub items: Vec<HomeItem>,
}

fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("BINDER_HOME_LAYOUT") {
        return PathBuf::from(path);
    }
    if let Some(root) = std::env::var_os("MOCHIOS_PREVIEW_CONFIG_ROOT") {
        return PathBuf::from(root).join("binder/tablet-home.json");
    }
    #[cfg(target_os = "mochios")]
    {
        PathBuf::from("/var/config/applications/org.mochios.binder/tablet-home.json")
    }
    #[cfg(not(target_os = "mochios"))]
    {
        PathBuf::from("/tmp/mochios-binder/tablet-home.json")
    }
}

impl HomeLayout {
    pub fn load() -> Self {
        fs::read_to_string(config_path())
            .ok()
            .and_then(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path();
        fs::create_dir_all(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, self.encode())?;
        fs::rename(temporary, path)
    }

    fn next_id(&self) -> u64 {
        self.items
            .iter()
            .map(|item| item.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn synchronize<'a>(&mut self, applications: impl Iterator<Item = &'a str>) -> bool {
        let mut known = HashSet::new();
        for item in &self.items {
            match &item.kind {
                ItemKind::Application(id) => {
                    known.insert(id.clone());
                }
                ItemKind::Folder { applications, .. } => known.extend(applications.iter().cloned()),
                ItemKind::Widget { .. } => {}
            }
        }
        let mut changed = false;
        for application in applications {
            if self.items.len() >= MAX_ITEMS {
                break;
            }
            if !known.insert(application.to_owned()) {
                continue;
            }
            let mut position = 0;
            while self.items.iter().any(|item| {
                item.slot
                    == Slot {
                        column: position / ROWS,
                        row: position % ROWS,
                    }
            }) {
                position += 1;
            }
            self.items.push(HomeItem {
                id: self.next_id(),
                slot: Slot {
                    column: position / ROWS,
                    row: position % ROWS,
                },
                kind: ItemKind::Application(application.to_owned()),
            });
            changed = true;
        }
        changed
    }

    pub fn move_item(&mut self, id: u64, slot: Slot, combine: bool) -> bool {
        if slot.row >= ROWS || slot.column >= 256 {
            return false;
        }
        let Some(source) = self.items.iter().position(|item| item.id == id) else {
            return false;
        };
        if self.items[source].slot == slot {
            return false;
        }
        let original = self.items[source].slot;
        if let Some(target) = self.items.iter().position(|item| item.slot == slot) {
            if combine {
                let ItemKind::Application(application) = self.items[source].kind.clone() else {
                    return false;
                };
                match &mut self.items[target].kind {
                    ItemKind::Application(other) => {
                        self.items[target].kind = ItemKind::Folder {
                            name: String::from("Folder"),
                            applications: vec![other.clone(), application],
                        };
                    }
                    ItemKind::Folder { applications, .. } if applications.len() < 64 => {
                        applications.push(application)
                    }
                    _ => return false,
                }
                self.items.remove(source);
                return true;
            }
            self.items[target].slot = original;
        }
        self.items[source].slot = slot;
        true
    }

    fn encode(&self) -> String {
        let items: Vec<_> = self.items.iter().map(|item| {
            let kind = match &item.kind {
                ItemKind::Application(id) => json!({"application": id}),
                ItemKind::Folder { name, applications } => json!({"folder": name, "applications": applications}),
                ItemKind::Widget { provider } => json!({"widget": provider}),
            };
            json!({"id": item.id, "column": item.slot.column, "row": item.slot.row, "kind": kind})
        }).collect();
        json!({"version":1,"items":items}).to_string()
    }

    fn parse(text: &str) -> Option<Self> {
        if text.len() > 256 * 1024 {
            return None;
        }
        let value: Value = serde_json::from_str(text).ok()?;
        if value["version"].as_u64()? != 1 {
            return None;
        }
        let values = value["items"].as_array()?;
        if values.len() > MAX_ITEMS {
            return None;
        }
        let mut items = Vec::new();
        let mut ids = HashSet::new();
        let mut slots = HashSet::new();
        let mut applications_seen = HashSet::new();
        for value in values {
            let id = value["id"].as_u64()?;
            let column = u32::try_from(value["column"].as_u64()?).ok()?;
            let row = u32::try_from(value["row"].as_u64()?).ok()?;
            if id == 0
                || id == u64::MAX
                || column >= 256
                || row >= ROWS
                || !ids.insert(id)
                || !slots.insert((column, row))
            {
                return None;
            }
            let kind = &value["kind"];
            let mut identity = |value: &Value| -> Option<String> {
                let id = value.as_str()?;
                if id.is_empty() || id.len() > 192 || !applications_seen.insert(id.to_owned()) {
                    return None;
                }
                Some(id.to_owned())
            };
            let kind = if let Some(id) = kind.get("application") {
                ItemKind::Application(identity(id)?)
            } else if let Some(name) = kind["folder"].as_str() {
                if name.len() > 96 {
                    return None;
                }
                let members = kind["applications"].as_array()?;
                if members.len() > 64 {
                    return None;
                }
                ItemKind::Folder {
                    name: name.to_owned(),
                    applications: members.iter().map(&mut identity).collect::<Option<_>>()?,
                }
            } else {
                ItemKind::Widget {
                    provider: kind["widget"].as_str()?.chars().take(192).collect(),
                }
            };
            items.push(HomeItem {
                id,
                slot: Slot { column, row },
                kind,
            });
        }
        Some(Self { items })
    }
}

pub(super) fn maximum_scroll(width: f32, pitch: f32, columns: u32) -> f32 {
    let content = (columns as f32 * pitch).max(width * 2.0);
    ((content - width) / (width * 0.5).max(1.0)).ceil() * width * 0.5
}

pub(super) fn resisted(position: f32, maximum: f32, width: f32) -> f32 {
    let edge = position.clamp(0.0, maximum);
    let excess = position - edge;
    edge + excess / (1.0 + excess.abs() / (width * 0.18).max(1.0)) * 0.4
}

pub(super) fn snap(position: f32, velocity: f32, width: f32, maximum: f32) -> f32 {
    let step = (width * 0.5).max(1.0);
    let projected = position + velocity.clamp(-width * 1.5, width * 1.5) * 0.08;
    (projected / step)
        .round()
        .mul_add(step, 0.0)
        .clamp(0.0, maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_canvas_snaps_in_half_viewports_and_preserves_edges() {
        assert_eq!(snap(310.0, 0.0, 1000.0, 2000.0), 500.0);
        assert_eq!(snap(700.0, 2200.0, 1000.0, 2000.0), 1000.0);
        assert_eq!(snap(-500.0, -1000.0, 1000.0, 2000.0), 0.0);
        assert_eq!(snap(2200.0, 1000.0, 1000.0, 2000.0), 2000.0);
        assert!(resisted(-100.0, 1000.0, 800.0) > -100.0);
        assert_eq!(maximum_scroll(800.0, 800.0 / 6.0, 12), 800.0);
    }
    #[test]
    fn folder_placement_round_trips_without_copying_app_metadata() {
        let mut layout = HomeLayout::default();
        layout.synchronize(["org.test.one", "org.test.two"].into_iter());
        assert!(layout.move_item(1, Slot { column: 0, row: 1 }, true));
        assert_eq!(layout.items.len(), 1);
        assert_eq!(HomeLayout::parse(&layout.encode()), Some(layout.clone()));
        assert!(!layout.synchronize(["org.test.one", "org.test.two"].into_iter()));
        assert!(layout.move_item(2, Slot { column: 8, row: 2 }, false));
        assert_eq!(HomeLayout::parse(&layout.encode()), Some(layout));
        assert!(HomeLayout::parse("{\"version\":2,\"items\":[]}").is_none());
    }
}
