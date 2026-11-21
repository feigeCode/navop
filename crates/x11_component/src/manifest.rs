use std::collections::BTreeMap;
use std::path::{Component, Path};

use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::X11ComponentError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub schema: u32,
    pub components: Components,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Components {
    #[serde(rename = "x11-server")]
    pub x11_server: X11Component,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct X11Component {
    pub channel: String,
    pub pinned_for_navop: String,
    #[serde(default)]
    pub rollout_percent: Option<u8>,
    #[serde(deserialize_with = "deserialize_artifacts")]
    pub artifacts: BTreeMap<String, Artifact>,
}

fn deserialize_artifacts<'de, D>(deserializer: D) -> Result<BTreeMap<String, Artifact>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ArtifactsVisitor;

    impl<'de> serde::de::Visitor<'de> for ArtifactsVisitor {
        type Value = BTreeMap<String, Artifact>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("不重复的 X11 target 映射")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut artifacts = BTreeMap::new();
            while let Some((target, artifact)) = map.next_entry::<String, Artifact>()? {
                if artifacts.insert(target.clone(), artifact).is_some() {
                    return Err(serde::de::Error::custom(format!("重复 target：{target}")));
                }
            }
            Ok(artifacts)
        }
    }

    deserializer.deserialize_map(ArtifactsVisitor)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    Binary,
    Zip,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub version: String,
    pub sha256: String,
    pub signature: String,
    pub min_vulkan: String,
    pub format: ArtifactFormat,
    pub executable_path: String,
}

impl Artifact {
    pub fn executable_relative_path(&self) -> &Path {
        Path::new(&self.executable_path)
    }
}

impl ComponentManifest {
    pub fn parse(text: &str, navop: &Version) -> Result<Self, X11ComponentError> {
        let manifest: Self = serde_json::from_str(text)
            .map_err(|error| X11ComponentError::InvalidManifest(error.to_string()))?;
        manifest.validate(navop)?;
        Ok(manifest)
    }

    pub fn artifact(&self, target: &str) -> Result<Artifact, X11ComponentError> {
        self.components
            .x11_server
            .artifacts
            .get(target)
            .cloned()
            .ok_or_else(|| X11ComponentError::MissingArtifact(target.into()))
    }

    pub fn rollout_percent(&self) -> u8 {
        self.components.x11_server.rollout_percent.unwrap_or(0)
    }

    fn validate(&self, navop: &Version) -> Result<(), X11ComponentError> {
        if self.schema != 1 {
            return invalid("schema 必须为 1");
        }
        let component = &self.components.x11_server;
        if component.channel != "stable" {
            return invalid("channel 必须为 stable");
        }
        if component
            .rollout_percent
            .is_some_and(|percent| percent > 100)
        {
            return invalid("rollout_percent 不能超过 100");
        }
        let requirement = VersionReq::parse(&component.pinned_for_navop)
            .map_err(|error| X11ComponentError::InvalidManifest(error.to_string()))?;
        if !requirement.matches(navop) {
            return invalid(format!("Navop {navop} 不满足 pinned_for_navop"));
        }
        for artifact in component.artifacts.values() {
            validate_artifact(artifact)?;
        }
        Ok(())
    }
}

fn validate_artifact(artifact: &Artifact) -> Result<(), X11ComponentError> {
    Version::parse(&artifact.version)
        .map_err(|error| X11ComponentError::InvalidManifest(error.to_string()))?;
    Version::parse(&artifact.min_vulkan)
        .map_err(|error| X11ComponentError::InvalidManifest(error.to_string()))?;
    validate_executable_path(&artifact.executable_path)?;
    let valid_sha = artifact.sha256.len() == 64
        && artifact
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if !valid_sha {
        return invalid("sha256 必须为 64 位小写十六进制");
    }
    Ok(())
}

fn validate_executable_path(path: &str) -> Result<(), X11ComponentError> {
    let valid = !path.is_empty()
        && !path.contains(['\\', '\0'])
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    if !valid {
        return invalid("executable_path 必须是安全的相对路径");
    }
    Ok(())
}

fn invalid<T>(reason: impl Into<String>) -> Result<T, X11ComponentError> {
    Err(X11ComponentError::InvalidManifest(reason.into()))
}

#[cfg(test)]
mod tests;
