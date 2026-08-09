use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

const DEFAULT_PINNED: [&str; 2] = ["org.mochios.files", "org.mochios.terminal"];

#[cfg(target_os = "mochios")]
const CONFIG_PATH: &str = "/libraries/applications/org.mochios.binder/dock.conf";

#[cfg(not(target_os = "mochios"))]
const CONFIG_PATH: &str = "/tmp/mochios-binder/dock.conf";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DockPreferences {
    pinned: Vec<String>,
}

impl Default for DockPreferences {
    fn default() -> Self {
        Self {
            pinned: DEFAULT_PINNED.iter().map(|id| String::from(*id)).collect(),
        }
    }
}

impl DockPreferences {
    pub(crate) fn load() -> Self {
        fs::read_to_string(CONFIG_PATH)
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub(crate) fn pinned(&self) -> &[String] {
        &self.pinned
    }

    pub(crate) fn is_pinned(&self, bundle_id: &str) -> bool {
        self.pinned.iter().any(|candidate| candidate == bundle_id)
    }

    pub(crate) fn pin(&mut self, bundle_id: &str) -> bool {
        if !valid_bundle_id(bundle_id) || self.is_pinned(bundle_id) {
            return false;
        }
        self.pinned.push(bundle_id.to_string());
        true
    }

    pub(crate) fn unpin(&mut self, bundle_id: &str) -> bool {
        let Some(index) = self
            .pinned
            .iter()
            .position(|candidate| candidate == bundle_id)
        else {
            return false;
        };
        self.pinned.remove(index);
        true
    }

    pub(crate) fn move_pin(&mut self, from: usize, to: usize) -> bool {
        if from >= self.pinned.len() || to >= self.pinned.len() || from == to {
            return false;
        }
        let bundle_id = self.pinned.remove(from);
        self.pinned.insert(to, bundle_id);
        true
    }

    pub(crate) fn move_pin_bundle(&mut self, from: &str, to: &str) -> bool {
        let Some(from_index) = self.pinned.iter().position(|candidate| candidate == from) else {
            return false;
        };
        let Some(to_index) = self.pinned.iter().position(|candidate| candidate == to) else {
            return false;
        };
        self.move_pin(from_index, to_index)
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        let path = Path::new(CONFIG_PATH);
        let Some(parent) = path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Dock config path",
            ));
        };
        match fs::create_dir(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let mut contents = self.pinned.join("\n");
        contents.push('\n');
        fs::write(path, contents)
    }

    fn parse(text: &str) -> Self {
        let mut seen = HashSet::new();
        let pinned = text
            .lines()
            .map(str::trim)
            .filter(|id| valid_bundle_id(id) && seen.insert((*id).to_string()))
            .map(String::from)
            .collect();
        Self { pinned }
    }
}

fn valid_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pins_files_before_terminal() {
        assert_eq!(
            DockPreferences::default().pinned(),
            ["org.mochios.files", "org.mochios.terminal"]
        );
    }

    #[test]
    fn parse_rejects_invalid_and_duplicate_ids() {
        let preferences = DockPreferences::parse(
            "org.mochios.files\n../escape\norg.mochios.files\norg.mochios.terminal\n",
        );
        assert_eq!(
            preferences.pinned(),
            ["org.mochios.files", "org.mochios.terminal"]
        );
    }

    #[test]
    fn pin_unpin_and_reorder_are_stable() {
        let mut preferences = DockPreferences::default();
        assert!(preferences.pin("org.mochios.viewkit-test"));
        assert!(preferences.move_pin(2, 0));
        assert!(preferences.move_pin_bundle("org.mochios.viewkit-test", "org.mochios.files"));
        assert!(preferences.unpin("org.mochios.terminal"));
        assert_eq!(
            preferences.pinned(),
            ["org.mochios.files", "org.mochios.viewkit-test"]
        );
    }
}
