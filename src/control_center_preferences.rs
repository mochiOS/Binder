use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(target_os = "mochios")]
const CONFIG_PATH: &str = ".config/mochios/control-center.conf";

#[cfg(not(target_os = "mochios"))]
const CONFIG_PATH: &str = "/tmp/mochios-binder/control-center.conf";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlCenterPreferences {
    order: Vec<String>,
    hidden: HashSet<String>,
}

impl ControlCenterPreferences {
    pub(crate) fn load() -> Self {
        fs::read_to_string(CONFIG_PATH)
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub(crate) fn ordered_ids<'a>(
        &self,
        available: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        let available = available.into_iter().collect::<Vec<_>>();
        let available_set = available.iter().copied().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut result = self
            .order
            .iter()
            .filter(|id| available_set.contains(id.as_str()) && seen.insert((*id).clone()))
            .cloned()
            .collect::<Vec<_>>();
        result.extend(
            available
                .into_iter()
                .filter(|id| seen.insert((*id).to_string()))
                .map(String::from),
        );
        result
    }

    pub(crate) fn is_hidden(&self, id: &str) -> bool {
        self.hidden.contains(id)
    }

    pub(crate) fn set_hidden(&mut self, id: &str, hidden: bool) -> bool {
        if hidden {
            self.hidden.insert(id.to_string())
        } else {
            self.hidden.remove(id)
        }
    }

    pub(crate) fn move_before(&mut self, from: &str, to: &str, available: &[String]) -> bool {
        if from == to {
            return false;
        }
        let mut order = self.ordered_ids(available.iter().map(String::as_str));
        let (Some(from_index), Some(to_index)) = (
            order.iter().position(|id| id == from),
            order.iter().position(|id| id == to),
        ) else {
            return false;
        };
        let id = order.remove(from_index);
        let adjusted = if from_index < to_index {
            to_index - 1
        } else {
            to_index
        };
        order.insert(adjusted, id);
        self.order = order;
        true
    }

    pub(crate) fn move_by(&mut self, id: &str, offset: isize, available: &[String]) -> bool {
        let mut order = self.ordered_ids(available.iter().map(String::as_str));
        let Some(index) = order.iter().position(|candidate| candidate == id) else {
            return false;
        };
        let target = index
            .saturating_add_signed(offset)
            .min(order.len().saturating_sub(1));
        if target == index {
            return false;
        }
        order.swap(index, target);
        self.order = order;
        true
    }

    pub(crate) fn reconcile(&mut self, available: &[String]) -> bool {
        let available = available.iter().map(String::as_str).collect::<HashSet<_>>();
        let previous_order_len = self.order.len();
        let previous_hidden_len = self.hidden.len();
        self.order.retain(|id| available.contains(id.as_str()));
        self.hidden.retain(|id| available.contains(id.as_str()));
        previous_order_len != self.order.len() || previous_hidden_len != self.hidden.len()
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        let path = Path::new(CONFIG_PATH);
        let Some(parent) = path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid config path",
            ));
        };
        fs::create_dir_all(parent)?;
        let mut contents = String::new();
        for id in &self.order {
            contents.push_str("item=");
            contents.push_str(id);
            contents.push('\n');
        }
        let mut hidden = self.hidden.iter().collect::<Vec<_>>();
        hidden.sort();
        for id in hidden {
            contents.push_str("hidden=");
            contents.push_str(id);
            contents.push('\n');
        }
        fs::write(path, contents)
    }

    fn parse(text: &str) -> Self {
        let mut preferences = Self::default();
        let mut ordered = HashSet::new();
        for line in text.lines().map(str::trim) {
            if let Some(id) = line.strip_prefix("item=") {
                if valid_id(id) && ordered.insert(id.to_string()) {
                    preferences.order.push(id.to_string());
                }
            } else if let Some(id) = line.strip_prefix("hidden=")
                && valid_id(id)
            {
                preferences.hidden.insert(id.to_string());
            }
        }
        preferences
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 192
        && id.bytes().all(|byte| {
            matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b':')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_order_and_hidden_items_strictly() {
        let preferences = ControlCenterPreferences::parse(
            "item=builtin.network\nitem=bad/path\nitem=builtin.network\nhidden=app:edit\n",
        );
        assert_eq!(preferences.order, ["builtin.network"]);
        assert!(preferences.is_hidden("app:edit"));
        assert!(!preferences.is_hidden("bad/path"));
    }

    #[test]
    fn appends_new_items_and_reorders_existing_items() {
        let mut preferences = ControlCenterPreferences::parse("item=b\nitem=a\n");
        let available = vec![String::from("a"), String::from("b"), String::from("c")];
        assert_eq!(
            preferences.ordered_ids(available.iter().map(String::as_str)),
            ["b", "a", "c"]
        );
        assert!(preferences.move_before("c", "b", &available));
        assert_eq!(
            preferences.ordered_ids(available.iter().map(String::as_str)),
            ["c", "b", "a"]
        );
        assert!(preferences.move_by("c", 1, &available));
        assert_eq!(
            preferences.ordered_ids(available.iter().map(String::as_str)),
            ["b", "c", "a"]
        );
        assert!(!preferences.move_by("a", 1, &available));
    }

    #[test]
    fn removes_preferences_for_uninstalled_items() {
        let mut preferences = ControlCenterPreferences::parse(
            "item=removed\nitem=kept\nhidden=removed\nhidden=kept\n",
        );
        assert!(preferences.reconcile(&[String::from("kept")]));
        assert_eq!(preferences.order, ["kept"]);
        assert!(!preferences.is_hidden("removed"));
        assert!(preferences.is_hidden("kept"));
    }
}
