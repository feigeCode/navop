use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellSurface {
    #[default]
    Tab,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShellHostModule {
    Context,
    Resource,
    Job,
    Event,
    Blob,
    Log,
    Runtime,
}

impl ShellHostModule {
    pub fn requires_backend(self) -> bool {
        matches!(self, Self::Resource | Self::Job | Self::Event | Self::Blob)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellViewContrib {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub surface: ShellSurface,
    #[serde(default)]
    pub singleton: bool,
    #[serde(default)]
    pub backends: BTreeMap<String, String>,
    #[serde(default)]
    pub modules: Vec<ShellHostModule>,
}
