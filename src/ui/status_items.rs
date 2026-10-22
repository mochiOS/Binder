//! One AppCore registry feeds menu entries, pinned icons and customization.
use crate::platform::AppInfo;
use appcore::{
    ApplicationStatusAction, StatusItem, StatusItemAction, StatusItemRegistry, SystemStatusItem,
};
use std::cell::RefCell;
use std::fs;
use viewkit::prelude::SymbolName;
const MAX_APP_ITEMS: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ItemAction {
    ToggleInput,
    ToggleFocus,
    ShowNetwork,
    ShowBattery,
    ShowNotifications,
    OpenSettings,
    Lock,
    OpenApplication(String),
    ShowApplicationCard { bundle_id: String, item_id: String },
}

#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) symbol: SymbolName,
    pub(crate) action: ItemAction,
    pub(crate) default_placement: appcore::StatusItemPlacement,
    pub(crate) source: String,
    registration: StatusItem,
}

#[derive(Default)]
struct CatalogCache {
    apps: Option<Vec<AppInfo>>,
    items: Vec<Item>,
}
thread_local! {static CATALOG:RefCell<CatalogCache>=RefCell::new(CatalogCache::default());}

fn from_registration(item: &StatusItem, source: &str) -> Result<Item, &'static str> {
    let symbol = SymbolName::from_asset_name(item.symbol()).ok_or("missing VK Symbol")?;
    let action = match item.action() {
        StatusItemAction::System(SystemStatusItem::InputMethod) => ItemAction::ToggleInput,
        StatusItemAction::System(SystemStatusItem::Notifications) => ItemAction::ShowNotifications,
        StatusItemAction::System(SystemStatusItem::Battery) => ItemAction::ShowBattery,
        StatusItemAction::System(SystemStatusItem::Network) => ItemAction::ShowNetwork,
        StatusItemAction::OpenApplication { bundle_id } => {
            ItemAction::OpenApplication(bundle_id.clone())
        }
        StatusItemAction::ShowCard { bundle_id, item_id } => ItemAction::ShowApplicationCard {
            bundle_id: bundle_id.clone(),
            item_id: item_id.clone(),
        },
    };
    Ok(Item {
        id: item.id().into(),
        title: item.title().into(),
        symbol,
        action,
        default_placement: item.default_placement(),
        source: source.into(),
        registration: item.clone(),
    })
}
fn system_registrations() -> StatusItemRegistry {
    let mut registry = StatusItemRegistry::new();
    for (kind, title, symbol) in [
        (
            SystemStatusItem::InputMethod,
            "Input Method",
            SymbolName::Keyboard,
        ),
        (
            SystemStatusItem::Notifications,
            "Notifications",
            SymbolName::Bell,
        ),
        (
            SystemStatusItem::Battery,
            "Battery",
            SymbolName::BatteryFull,
        ),
        (SystemStatusItem::Network, "Wi-Fi", SymbolName::Wifi),
    ] {
        registry
            .register(
                StatusItem::system(kind, title, symbol.asset_name()).expect("system registration"),
            )
            .expect("system registry capacity");
    }
    registry
}
pub(crate) fn catalog(apps: &[AppInfo]) -> Vec<Item> {
    CATALOG.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.apps.as_deref() != Some(apps) {
            let mut registry = system_registrations();
            for item in apps.iter().flat_map(read_app_items) {
                if let Err(error) = registry.register(item.registration) {
                    eprintln!("status item registration rejected: {error:?}");
                }
            }
            let items = registry
                .items()
                .map(|entry| {
                    let source = apps
                        .iter()
                        .find(|app| app.bundle_id == entry.source())
                        .map(|app| app.name.as_str())
                        .unwrap_or("mochiOS");
                    from_registration(entry, source).expect("validated VK symbol")
                })
                .collect();
            cache.apps = Some(apps.to_vec());
            cache.items = items;
        }
        cache.items.clone()
    })
}
pub(crate) fn read_app_items(app: &AppInfo) -> Vec<Item> {
    let Ok(text) = fs::read_to_string(app.root.join("manifest.toml")) else {
        return Vec::new();
    };
    parse_app_items(&text, app).unwrap_or_else(|error| {
        eprintln!(
            "ignored invalid status registration for {}: {error}",
            app.bundle_id
        );
        Vec::new()
    })
}
pub(crate) fn parse_app_items(text: &str, app: &AppInfo) -> Result<Vec<Item>, &'static str> {
    let valid_format = text.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(key, value)| key.trim() == "format" && value.trim() == "1")
    });
    if !valid_format {
        return Err("unsupported format");
    }
    let mut result = Vec::new();
    for section in text
        .split("[[application.control_center_items]]")
        .skip(1)
        .take(MAX_APP_ITEMS + 1)
    {
        if result.len() == MAX_APP_ITEMS {
            return Err("too many items");
        }
        let field = |name: &str| -> Option<&str> {
            section.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                if key.trim() != name {
                    return None;
                }
                value.trim().strip_prefix('"')?.strip_suffix('"')
            })
        };
        let id = field("id").ok_or("missing id")?;
        let title = field("title").ok_or("missing title")?;
        let symbol = field("symbol")
            .and_then(SymbolName::from_asset_name)
            .ok_or("invalid symbol")?;
        if !valid_local_id(id) || title.is_empty() || title.len() > 40 {
            return Err("invalid item fields");
        }
        let action = match field("action") {
            Some("open-application") => ItemAction::OpenApplication(app.bundle_id.clone()),
            Some("show-card") => ItemAction::ShowApplicationCard {
                bundle_id: app.bundle_id.clone(),
                item_id: id.to_string(),
            },
            _ => return Err("invalid item fields"),
        };
        let app_action = match action {
            ItemAction::OpenApplication(_) => ApplicationStatusAction::OpenApplication,
            _ => ApplicationStatusAction::ShowCard,
        };
        let registration =
            StatusItem::application(&app.bundle_id, id, title, symbol.asset_name(), app_action)
                .map_err(|_| "invalid registration")?;
        result.push(from_registration(&registration, &app.name)?);
    }
    Ok(result)
}

fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_are_registered_through_appcore_and_keep_vk_assets() {
        let items = catalog(&[]);
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            [
                "builtin.input",
                "builtin.notifications",
                "builtin.battery",
                "builtin.network"
            ]
        );
        assert_eq!(items[2].symbol, SymbolName::BatteryFull);
        assert_eq!(items[3].symbol, SymbolName::Wifi);
        assert!(
            items
                .iter()
                .all(|i| i.default_placement == appcore::StatusItemPlacement::Pinned)
        );
    }
}
