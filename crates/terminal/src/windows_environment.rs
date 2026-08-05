use std::collections::HashMap;

#[cfg(target_os = "windows")]
#[path = "windows_environment_windows.rs"]
mod platform;
#[cfg(target_os = "windows")]
pub(crate) use platform::refreshed_windows_environment;

#[derive(Clone, Debug)]
pub(crate) struct PersistentEnvironmentValue {
    name: String,
    value: String,
    expandable: bool,
}

impl PersistentEnvironmentValue {
    pub(crate) fn new(name: &str, value: &str, expandable: bool) -> Self {
        Self {
            name: name.to_string(),
            value: value.to_string(),
            expandable,
        }
    }
}

pub(crate) fn merge_persistent_environment(
    system: Vec<PersistentEnvironmentValue>,
    user: Vec<PersistentEnvironmentValue>,
    inherited: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut merged = inherited
        .into_iter()
        .map(|(name, value)| PersistentEnvironmentValue::new(&name, &value, false))
        .collect::<Vec<_>>();

    for value in system {
        set_environment_value(&mut merged, value);
    }

    for value in user {
        if value.name.eq_ignore_ascii_case("Path") {
            append_path_value(&mut merged, value);
        } else {
            set_environment_value(&mut merged, value);
        }
    }

    let raw_values = merged
        .iter()
        .map(|value| (value.name.to_ascii_lowercase(), value.value.clone()))
        .collect::<HashMap<_, _>>();

    merged
        .into_iter()
        .map(|entry| {
            let value = if entry.expandable {
                expand_environment_value(&entry.value, &raw_values)
            } else {
                entry.value
            };
            (entry.name, value)
        })
        .collect()
}

pub(crate) fn merge_environment_overrides(
    mut base: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
) -> Vec<(String, String)> {
    for (name, value) in overrides {
        if let Some(existing) = base
            .iter_mut()
            .find(|(existing_name, _)| existing_name.eq_ignore_ascii_case(&name))
        {
            *existing = (name, value);
        } else {
            base.push((name, value));
        }
    }
    base
}

pub(crate) fn environment_value<'a>(
    environment: &'a [(String, String)],
    name: &str,
) -> Option<&'a str> {
    environment.iter().find_map(|(existing_name, value)| {
        existing_name
            .eq_ignore_ascii_case(name)
            .then_some(value.as_str())
    })
}

fn set_environment_value(
    environment: &mut Vec<PersistentEnvironmentValue>,
    value: PersistentEnvironmentValue,
) {
    if let Some(existing) = environment
        .iter_mut()
        .find(|existing| existing.name.eq_ignore_ascii_case(&value.name))
    {
        *existing = value;
    } else {
        environment.push(value);
    }
}

fn append_path_value(
    environment: &mut Vec<PersistentEnvironmentValue>,
    mut value: PersistentEnvironmentValue,
) {
    if let Some(existing) = environment
        .iter_mut()
        .find(|existing| existing.name.eq_ignore_ascii_case("Path"))
    {
        if !existing.value.is_empty() && !value.value.is_empty() {
            existing.value.push(';');
        }
        existing.value.push_str(&value.value);
        existing.expandable |= value.expandable;
    } else {
        value.name = "Path".to_string();
        set_environment_value(environment, value);
    }
}

fn expand_environment_value(value: &str, environment: &HashMap<String, String>) -> String {
    let mut expanded = value.to_string();
    for _ in 0..16 {
        let next = replace_environment_references(&expanded, environment);
        if next == expanded || next.len() > 32 * 1024 {
            break;
        }
        expanded = next;
    }
    expanded
}

fn replace_environment_references(value: &str, environment: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remainder = value;

    while let Some(start) = remainder.find('%') {
        result.push_str(&remainder[..start]);
        let after_start = &remainder[start + 1..];
        let Some(end) = after_start.find('%') else {
            result.push_str(&remainder[start..]);
            return result;
        };
        let name = &after_start[..end];
        let reference = &after_start[end + 1..];
        if name.is_empty() {
            result.push('%');
        } else if let Some(replacement) = environment.get(&name.to_ascii_lowercase()) {
            result.push_str(replacement);
        } else {
            result.push('%');
            result.push_str(name);
            result.push('%');
        }
        remainder = reference;
    }

    result.push_str(remainder);
    result
}

#[cfg(test)]
mod tests {
    use super::{
        PersistentEnvironmentValue, environment_value, expand_environment_value,
        merge_environment_overrides, merge_persistent_environment,
    };
    use std::collections::HashMap;

    fn plain(name: &str, value: &str) -> PersistentEnvironmentValue {
        PersistentEnvironmentValue::new(name, value, false)
    }

    fn expandable(name: &str, value: &str) -> PersistentEnvironmentValue {
        PersistentEnvironmentValue::new(name, value, true)
    }

    #[test]
    fn persistent_environment_appends_user_path_and_overrides_other_values() {
        let system = vec![
            plain("Path", r"C:\Windows\System32"),
            plain("Shared", "system"),
        ];
        let user = vec![plain("PATH", r"C:\Tools"), plain("SHARED", "user")];

        let merged = merge_persistent_environment(system, user, vec![]);

        assert_eq!(
            Some(r"C:\Windows\System32;C:\Tools"),
            environment_value(&merged, "PATH")
        );
        assert_eq!(Some("user"), environment_value(&merged, "shared"));
    }

    #[test]
    fn persistent_environment_expands_against_refreshed_and_inherited_values() {
        let system = vec![
            plain("SystemRoot", r"C:\Windows"),
            expandable("Tools", r"%SystemRoot%\Tools"),
            expandable("Path", r"%SystemRoot%\System32"),
        ];
        let user = vec![expandable("PATH", r"%Tools%\bin;%USERPROFILE%\bin")];
        let inherited = vec![("USERPROFILE".to_string(), r"C:\Users\Admin".to_string())];

        let merged = merge_persistent_environment(system, user, inherited);

        assert_eq!(
            Some(r"C:\Windows\System32;C:\Windows\Tools\bin;C:\Users\Admin\bin"),
            environment_value(&merged, "path")
        );
    }

    #[test]
    fn explicit_environment_overrides_refreshed_values_case_insensitively() {
        let refreshed = vec![
            ("Path".to_string(), "registry".to_string()),
            ("KEEP".to_string(), "value".to_string()),
        ];
        let explicit = vec![
            ("PATH".to_string(), "configured".to_string()),
            ("EXTRA".to_string(), "enabled".to_string()),
        ];

        let merged = merge_environment_overrides(refreshed, explicit);

        assert_eq!(Some("configured"), environment_value(&merged, "path"));
        assert_eq!(Some("value"), environment_value(&merged, "keep"));
        assert_eq!(Some("enabled"), environment_value(&merged, "extra"));
        assert_eq!(
            1,
            merged
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("PATH"))
                .count()
        );
    }

    #[test]
    fn expansion_preserves_unknown_references() {
        let environment = HashMap::new();

        assert_eq!(
            r"%MISSING%\bin",
            expand_environment_value(r"%MISSING%\bin", &environment)
        );
    }

    #[test]
    fn cyclic_expansion_is_bounded() {
        let environment = HashMap::from([
            ("first".to_string(), "%SECOND%".to_string()),
            ("second".to_string(), "%FIRST%".to_string()),
        ]);

        let expanded = expand_environment_value("%FIRST%", &environment);

        assert!(expanded == "%FIRST%" || expanded == "%SECOND%");
    }
}
