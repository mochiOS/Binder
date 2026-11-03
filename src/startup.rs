//! Declarative setup startup. No application implementation is known to Binder.
use crate::platform::AppInfo;
use serde_json::Value;
use std::{fs::File, io::Read};
const MAX_BYTES: usize = 64 * 1024;
fn read(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    File::open(path)
        .and_then(|file| file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Startup policy is too large".into());
    }
    Ok(bytes)
}
fn bundle(value: &Value) -> Result<&str, String> {
    let value = value.as_str().ok_or("Missing application identity")?;
    if value.len() > 192
        || !value.contains('.')
        || value.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        return Err("Invalid application identity".into());
    }
    Ok(value)
}
fn select(
    policy: &[u8],
    installed: &[&str],
    requirements: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<Option<String>, String> {
    let policy: Value = serde_json::from_slice(policy).map_err(|e| e.to_string())?;
    if policy.get("format").and_then(Value::as_u64) != Some(1) {
        return Err("Unknown startup policy".into());
    }
    let rules = policy
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("Missing startup rules")?;
    if rules.len() > 16 {
        return Err("Too many startup rules".into());
    }
    for rule in rules {
        let handler = bundle(&rule["handler"])?;
        let path = rule["requirements"]
            .as_str()
            .ok_or("Missing requirements path")?;
        if !path.starts_with("/system/applications/")
            || path.contains('\\')
            || path
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err("Unsafe requirements path".into());
        }
        if !installed.contains(&handler) {
            continue;
        }
        let bytes = requirements(path)?;
        let required: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if required["format"].as_u64() != Some(1) {
            return Err("Unknown application setup plan".into());
        }
        let apps = required["applications"]
            .as_array()
            .ok_or("Missing setup applications")?;
        if apps.len() > 32 {
            return Err("Too many setup applications".into());
        }
        let mut missing = false;
        for app in apps {
            missing |= !installed.contains(&bundle(&app["bundle_id"])?);
        }
        if missing {
            return Ok(Some(handler.to_owned()));
        }
    }
    Ok(None)
}
pub(crate) fn application(apps: &[AppInfo]) -> Option<AppInfo> {
    #[cfg(target_os = "mochios")]
    let path = "/system/setup/session-startup.json".to_owned();
    #[cfg(not(target_os = "mochios"))]
    let path = std::env::var("MOCHIOS_PREVIEW_STARTUP_CONFIG").ok()?;
    let result = read(&path).and_then(|policy| {
        select(
            &policy,
            &apps
                .iter()
                .map(|app| app.bundle_id.as_str())
                .collect::<Vec<_>>(),
            read,
        )
    });
    match result {
        Ok(Some(id)) => apps.iter().find(|app| app.bundle_id == id).cloned(),
        Ok(None) => None,
        Err(error) => {
            eprintln!("Setup startup unavailable: {error}");
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const POLICY:&[u8]=br#"{"format":1,"rules":[{"handler":"org.example.store","requirements":"/system/applications/Store.app/setup.json"}]}"#;
    const APPS: &[u8] =
        br#"{"format":1,"applications":[{"bundle_id":"org.example.browser","name":"Browser"}]}"#;
    #[test]
    fn only_missing_apps_schedule_the_installed_handler() {
        assert_eq!(
            select(POLICY, &["org.example.store"], |_| Ok(APPS.to_vec())).unwrap(),
            Some("org.example.store".into())
        );
        assert_eq!(
            select(
                POLICY,
                &["org.example.store", "org.example.browser"],
                |_| Ok(APPS.to_vec())
            )
            .unwrap(),
            None
        );
        assert_eq!(select(POLICY, &[], |_| panic!()).unwrap(), None);
    }
    #[test]
    fn invalid_policy_and_read_failures_never_launch_a_handler() {
        assert!(select(b"{}", &[], |_| panic!()).is_err());
        assert!(
            select(
                POLICY,
                &["org.example.store"],
                |_| Err("read failed".into())
            )
            .is_err()
        );
        let unsafe_path = String::from_utf8(POLICY.to_vec()).unwrap().replace(
            "/system/applications/Store.app/setup.json",
            "/system/applications/../setup.json",
        );
        assert!(select(unsafe_path.as_bytes(), &["org.example.store"], |_| panic!()).is_err());
    }
}
