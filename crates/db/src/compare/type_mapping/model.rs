/// Describes how safely a source type can be represented by a target database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCompatibility {
    Exact,
    Equivalent,
    Widening,
    Lossy,
    Unsupported,
}

impl TypeCompatibility {
    pub fn is_safe_for_automatic_sync(self) -> bool {
        matches!(self, Self::Exact | Self::Equivalent | Self::Widening)
    }
}

/// Result of mapping a source column type into a target database dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedColumnType {
    pub target_type: String,
    pub compatibility: TypeCompatibility,
    pub warning: Option<String>,
}

impl MappedColumnType {
    pub(super) fn new(target_type: impl Into<String>, compatibility: TypeCompatibility) -> Self {
        Self {
            target_type: target_type.into(),
            compatibility,
            warning: None,
        }
    }

    pub(super) fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warning = Some(warning.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CanonicalColumnType {
    Boolean,
    Integer {
        bits: u16,
        unsigned: bool,
    },
    Decimal {
        precision: Option<u32>,
        scale: Option<u32>,
    },
    Float {
        bits: u16,
    },
    Character {
        varying: bool,
        length: Option<u32>,
        national: bool,
    },
    Text {
        national: bool,
    },
    Binary {
        fixed: bool,
        length: Option<u32>,
    },
    BitString {
        varying: bool,
        length: Option<u32>,
    },
    Date,
    Time {
        precision: Option<u32>,
        with_timezone: bool,
    },
    DateTime {
        precision: Option<u32>,
        with_timezone: bool,
    },
    Json,
    Uuid,
    RowVersion,
}

impl CanonicalColumnType {
    pub(super) fn supports_alias_equivalence(&self) -> bool {
        !matches!(
            self,
            Self::Text { .. } | Self::Binary { .. } | Self::RowVersion
        )
    }
}

#[derive(Debug)]
pub(super) struct ParsedTypeDeclaration {
    pub base: String,
    pub args: Vec<String>,
    pub unsigned: bool,
}
