use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuContrib {
    pub command: MenuCommandRef,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default = "default_requires_active")]
    pub requires_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MenuCommandRef {
    pub id: String,
}

impl<'de> Deserialize<'de> for MenuCommandRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let id = String::deserialize(deserializer)?;
        Ok(Self { id })
    }
}

fn default_requires_active() -> bool {
    true
}
