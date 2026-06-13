use std::collections::HashSet;

/// Support state for one external driver method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodSupport {
    /// The driver explicitly declares this method.
    Supported,
    /// The driver declares a method set, but this method is absent.
    Unsupported,
    /// The driver does not declare methods, so keep legacy behavior and try it.
    UnknownLegacy,
}

/// Effective method support set for one driver instance.
#[derive(Debug, Clone, Default)]
pub struct MethodSet {
    methods: Option<HashSet<String>>,
}

impl MethodSet {
    pub fn resolve(init: &[String], manifest: &[String]) -> Self {
        if !init.is_empty() {
            Self {
                methods: Some(init.iter().cloned().collect()),
            }
        } else {
            Self::from_manifest(manifest)
        }
    }

    pub fn from_manifest(methods: &[String]) -> Self {
        if methods.is_empty() {
            Self::legacy()
        } else {
            Self {
                methods: Some(methods.iter().cloned().collect()),
            }
        }
    }

    pub fn legacy() -> Self {
        Self { methods: None }
    }

    pub fn support(&self, method: &str) -> MethodSupport {
        match &self.methods {
            None => MethodSupport::UnknownLegacy,
            Some(methods) if methods.contains(method) => MethodSupport::Supported,
            Some(_) => MethodSupport::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn methods(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn empty_manifest_methods_use_legacy_attempts() {
        let set = MethodSet::from_manifest(&[]);

        assert_eq!(
            MethodSupport::UnknownLegacy,
            set.support("metadata.list_databases")
        );
    }

    #[test]
    fn declared_manifest_methods_gate_unsupported_requests() {
        let set = MethodSet::from_manifest(&methods(&["metadata.list_databases"]));

        assert_eq!(
            MethodSupport::Supported,
            set.support("metadata.list_databases")
        );
        assert_eq!(
            MethodSupport::Unsupported,
            set.support("metadata.list_columns")
        );
    }
}
