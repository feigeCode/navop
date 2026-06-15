use db::ipc::IpcDriverManifest;
use parking_lot::RwLock;
use serde_yaml::Value;
use std::collections::HashMap;

pub struct DriverI18nManager {
    cache: RwLock<HashMap<String, HashMap<String, Value>>>,
}

impl DriverI18nManager {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn translate(&self, driver: &IpcDriverManifest, key: &str) -> String {
        let locale = rust_i18n::locale();
        let locale = locale.as_ref();

        if let Some(text) = self.cached_translation(&driver.id, locale, key) {
            return text;
        }

        match driver.load_locale(locale) {
            Ok(translations) => {
                let result = lookup_nested_key(&translations, key).unwrap_or_else(|| key.into());
                self.cache
                    .write()
                    .entry(driver.id.clone())
                    .or_default()
                    .insert(locale.to_string(), translations);
                result
            }
            Err(error) => {
                tracing::warn!(
                    "failed to load locale '{}' for driver '{}': {}",
                    locale,
                    driver.id,
                    error
                );
                key.to_string()
            }
        }
    }

    fn cached_translation(&self, driver_id: &str, locale: &str, key: &str) -> Option<String> {
        let cache = self.cache.read();
        let translations = cache.get(driver_id)?.get(locale)?;
        lookup_nested_key(translations, key)
    }
}

fn lookup_nested_key(value: &Value, key: &str) -> Option<String> {
    if let Value::Mapping(map) = value {
        if let Some(Value::String(text)) = map.get(Value::String(key.to_string())) {
            return Some(text.clone());
        }
    }

    let mut current = value;
    for segment in key.split('.') {
        match current {
            Value::Mapping(map) => {
                current = map.get(Value::String(segment.to_string()))?;
            }
            _ => return None,
        }
    }

    match current {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

static DRIVER_I18N: once_cell::sync::Lazy<DriverI18nManager> =
    once_cell::sync::Lazy::new(DriverI18nManager::new);

pub fn t_driver(driver: &IpcDriverManifest, key: &str) -> String {
    DRIVER_I18N.translate(driver, key)
}

#[cfg(test)]
mod tests {
    use super::lookup_nested_key;

    #[test]
    fn lookup_nested_key_supports_nested_and_flat_keys() {
        let value: serde_yaml::Value = serde_yaml::from_str(
            r#"
connection:
  title: "Connect"
"database.connection.field.name": "Connection Name"
"#,
        )
        .unwrap();

        assert_eq!(
            Some("Connect".to_string()),
            lookup_nested_key(&value, "connection.title")
        );
        assert_eq!(
            Some("Connection Name".to_string()),
            lookup_nested_key(&value, "database.connection.field.name")
        );
        assert_eq!(None, lookup_nested_key(&value, "missing.key"));
    }
}
