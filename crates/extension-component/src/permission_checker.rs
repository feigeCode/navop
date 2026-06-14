//! 权限检查器。
//!
//! 在 Component Runtime 中拦截 host imports 调用，检查扩展是否拥有相应权限。

use anyhow::{Result, anyhow};

use crate::SqlAccess;
use crate::permissions::PermissionSet;

/// 权限检查器。
///
/// 封装 `PermissionSet`，提供方法级别的权限检查。
pub struct PermissionChecker {
    extension_id: String,
    permissions: PermissionSet,
}

impl PermissionChecker {
    pub fn new(extension_id: String, permissions: PermissionSet) -> Self {
        Self {
            extension_id,
            permissions,
        }
    }

    /// 检查数据库访问权限。
    pub fn check_db_access(&self, access: SqlAccess, connection_id: &str) -> Result<()> {
        if self.permissions.allows_db(access, connection_id) {
            Ok(())
        } else {
            Err(anyhow!(
                "extension '{}' lacks db:{:?} permission for connection '{}'",
                self.extension_id,
                access,
                connection_id
            ))
        }
    }

    /// 检查 UI 权限。
    pub fn check_ui(&self, permission: &str) -> Result<()> {
        if self.permissions.allows_ui(permission) {
            Ok(())
        } else {
            Err(anyhow!(
                "extension '{}' lacks ui permission: {}",
                self.extension_id,
                permission
            ))
        }
    }

    /// 检查连接列表权限。
    pub fn check_connection_list(&self) -> Result<()> {
        if self.permissions.allows_connection_list() {
            Ok(())
        } else {
            Err(anyhow!(
                "extension '{}' lacks db:connections:list permission",
                self.extension_id
            ))
        }
    }

    /// 检查存储权限。
    pub fn check_storage(&self, scope: &str, write: bool) -> Result<()> {
        // 存储权限可能以 storage: 或 ui: 开头
        let storage_perm = if write {
            format!("storage:write:{}", scope)
        } else {
            format!("storage:read:{}", scope)
        };

        let ui_perm = if write {
            format!("ui:storage:write:{}", scope)
        } else {
            format!("ui:storage:read:{}", scope)
        };

        if self.permissions.allows_ui(&storage_perm) || self.permissions.allows_ui(&ui_perm) {
            Ok(())
        } else {
            Err(anyhow!(
                "extension '{}' lacks storage permission for scope '{}' (write={})",
                self.extension_id,
                scope,
                write
            ))
        }
    }

    /// 检查通知权限。
    pub fn check_notify(&self) -> Result<()> {
        // notifications:show 属于 ui 命名空间
        if self.permissions.allows_ui("notifications:show")
            || self.permissions.allows_ui("ui:notify")
        {
            Ok(())
        } else {
            Err(anyhow!(
                "extension '{}' lacks notifications:show or ui:notify permission",
                self.extension_id
            ))
        }
    }

    /// 检查对话框权限。
    pub fn check_dialog(&self) -> Result<()> {
        self.check_ui("ui:dialog")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checker_allows_granted_db_permission() {
        let permissions = PermissionSet::new(["db:read:conn1", "db:write:conn2"]);
        let checker = PermissionChecker::new("test-ext".into(), permissions);

        assert!(checker.check_db_access(SqlAccess::Read, "conn1").is_ok());
        assert!(checker.check_db_access(SqlAccess::Write, "conn2").is_ok());
    }

    #[test]
    fn checker_denies_missing_db_permission() {
        let permissions = PermissionSet::new(["db:read:conn1"]);
        let checker = PermissionChecker::new("test-ext".into(), permissions);

        let result = checker.check_db_access(SqlAccess::Write, "conn1");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("lacks db:Write"));
    }

    #[test]
    fn checker_allows_granted_ui_permission() {
        let permissions = PermissionSet::new(["ui:notify", "ui:dialog"]);
        let checker = PermissionChecker::new("test-ext".into(), permissions);

        assert!(checker.check_notify().is_ok());
        assert!(checker.check_dialog().is_ok());
    }

    #[test]
    fn checker_denies_missing_ui_permission() {
        let permissions = PermissionSet::new(["ui:notify"]);
        let checker = PermissionChecker::new("test-ext".into(), permissions);

        let result = checker.check_dialog();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("lacks ui permission")
        );
    }

    #[test]
    fn checker_validates_connection_list_permission() {
        let with_perm = PermissionSet::new(["db:connections:list"]);
        let without_perm = PermissionSet::new(["db:read:conn1"]);

        let checker_with = PermissionChecker::new("test-ext".into(), with_perm);
        let checker_without = PermissionChecker::new("test-ext".into(), without_perm);

        assert!(checker_with.check_connection_list().is_ok());
        assert!(checker_without.check_connection_list().is_err());
    }

    #[test]
    fn checker_validates_storage_permissions() {
        let permissions =
            PermissionSet::new(["ui:storage:read:global", "ui:storage:write:workspace"]);
        let checker = PermissionChecker::new("test-ext".into(), permissions);

        assert!(checker.check_storage("global", false).is_ok());
        assert!(checker.check_storage("workspace", true).is_ok());
        assert!(checker.check_storage("global", true).is_err());
        assert!(checker.check_storage("user", false).is_err());
    }
}
