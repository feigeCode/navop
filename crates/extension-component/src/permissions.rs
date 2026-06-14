use std::collections::BTreeSet;

use crate::SqlAccess;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    db: BTreeSet<DbPermission>,
    ui: BTreeSet<String>,
    connection_list: bool,
}

impl PermissionSet {
    pub fn new<I, S>(permissions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = Self::default();
        for permission in permissions {
            set.add(permission.as_ref());
        }
        set
    }

    pub fn allows_db(&self, access: SqlAccess, connection_id: &str) -> bool {
        self.db
            .iter()
            .any(|permission| permission.matches(access, connection_id))
    }

    pub fn allows_ui(&self, permission: &str) -> bool {
        self.ui.contains(permission)
    }

    pub fn allows_connection_list(&self) -> bool {
        self.connection_list
    }

    fn add(&mut self, permission: &str) {
        if permission == "db:connections:list" {
            self.connection_list = true;
        } else if let Some(db) = DbPermission::parse(permission) {
            self.db.insert(db);
        } else if permission.starts_with("ui:") {
            self.ui.insert(permission.to_string());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DbPermission {
    access: DbPermissionAccess,
    connection_id: String,
}

impl DbPermission {
    fn parse(permission: &str) -> Option<Self> {
        let mut parts = permission.split(':');
        if parts.next()? != "db" {
            return None;
        }
        let access = DbPermissionAccess::parse(parts.next()?)?;
        let connection_id = parts.next()?.to_string();
        parts.next().is_none().then_some(Self {
            access,
            connection_id,
        })
    }

    fn matches(&self, access: SqlAccess, connection_id: &str) -> bool {
        self.access.matches(access)
            && (self.connection_id == "*" || self.connection_id == connection_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DbPermissionAccess {
    Read,
    Write,
    Schema,
    Admin,
}

impl DbPermissionAccess {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "schema" => Some(Self::Schema),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    fn matches(self, access: SqlAccess) -> bool {
        self.rank() >= access.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Read => 0,
            Self::Write => 1,
            Self::Schema => 2,
            Self::Admin => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_permission_matches_access_and_connection_scope() {
        let permissions = PermissionSet::new(["db:read:conn1"]);

        assert!(permissions.allows_db(SqlAccess::Read, "conn1"));
        assert!(!permissions.allows_db(SqlAccess::Read, "conn2"));
        assert!(!permissions.allows_db(SqlAccess::Write, "conn1"));
    }

    #[test]
    fn higher_database_permission_allows_lower_risk_access() {
        let permissions = PermissionSet::new(["db:admin:conn1", "db:schema:conn2"]);

        assert!(permissions.allows_db(SqlAccess::Read, "conn1"));
        assert!(permissions.allows_db(SqlAccess::Write, "conn1"));
        assert!(permissions.allows_db(SqlAccess::Schema, "conn1"));
        assert!(permissions.allows_db(SqlAccess::Admin, "conn1"));
        assert!(permissions.allows_db(SqlAccess::Read, "conn2"));
        assert!(permissions.allows_db(SqlAccess::Schema, "conn2"));
        assert!(!permissions.allows_db(SqlAccess::Admin, "conn2"));
    }

    #[test]
    fn connection_list_permission_is_explicit() {
        let permissions = PermissionSet::new(["db:connections:list", "db:read:conn1"]);

        assert!(permissions.allows_connection_list());
        assert!(!PermissionSet::new(["db:read:conn1"]).allows_connection_list());
    }
}
