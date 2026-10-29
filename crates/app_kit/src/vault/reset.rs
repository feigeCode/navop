//! 重置主密钥：清本地业务数据 → 清密钥文件 → 清笔记 / 云端 → 回到全新安装。
//!
//! 弹窗在 [`super::dialogs::show_reset_master_key_dialog`]；这里只做流程。云端清空
//! 与笔记目录依赖宿主产品，经 [`crate::vault::VaultHostHooks`] 注入，独立产品没有
//! 云同步 / 笔记时钩子为 no-op，只清本地。

use gpui::{App, Global, ParentElement as _, Styled as _, div};
use gpui_component::switch::Switch;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use one_core::crypto;
use one_core::master_key_meta;
use one_core::settings::AppSettings;
use one_core::storage::GlobalStorageState;

use crate::vault::{CloudWipeScope, host_hooks};

/// 主密钥固定必清的本地表：连接、钥匙串、分组与各类同步缓存。
///
/// 这些数据要么本身是密文，要么是密文派生出来的索引，保留它们等于留下一份
/// 永远解不开的数据，因此不提供"保留"选项。
const REQUIRED_LOCAL_TABLES: &[&str] = &[
    "connections",
    "credential_entries",
    "workspaces",
    "team_key_cache",
    "team_membership_cache",
    "personal_sync_conflicts",
    "personal_sync_status",
    "pending_cloud_deletions",
];

/// 重置范围：勾选 = 删除该项，默认全部勾选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetScope {
    /// 云端团队共享数据（团队密钥加密，主密钥重置不影响其可读性）
    pub team_data: bool,
    /// 查询历史
    pub query_history: bool,
    /// 命令历史与快捷命令
    pub command_history: bool,
    /// 笔记（磁盘文件）
    pub notes: bool,
    /// 云同步数据与版本信息
    pub cloud_sync: bool,
}

impl Default for ResetScope {
    fn default() -> Self {
        Self {
            team_data: true,
            query_history: true,
            command_history: true,
            notes: true,
            cloud_sync: true,
        }
    }
}

impl ResetScope {
    /// 云端要清空的范围：云同步数据跟随 `cloud_sync`，团队数据跟随 `team_data`。
    pub fn cloud_wipe_scope(self) -> CloudWipeScope {
        CloudWipeScope {
            personal_sync: self.cloud_sync,
            team_data: self.team_data,
        }
    }
}

/// 重置范围需要在设置页勾选、在入口处读取，因此放进全局状态。
pub struct GlobalResetScope {
    pub scope: ResetScope,
}

impl Global for GlobalResetScope {}

pub fn reset_scope(cx: &App) -> ResetScope {
    cx.try_global::<GlobalResetScope>()
        .map(|state| state.scope)
        .unwrap_or_default()
}

pub fn set_reset_scope(scope: ResetScope, cx: &mut App) {
    if !cx.has_global::<GlobalResetScope>() {
        cx.set_global(GlobalResetScope { scope });
    }
    cx.global_mut::<GlobalResetScope>().scope = scope;
}

/// 重置结果：分别记录清掉与没能清掉的数据，弹窗据此选择提示文案。
pub struct ResetOutcome {
    pub deleted: Vec<String>,
    pub failed: Vec<String>,
}

/// 执行重置：清本地（按勾选）→ 清密钥文件 → 清笔记 / 云端。
pub fn reset_master_key_data(scope: ResetScope, cx: &mut App) -> Result<ResetOutcome, String> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let connection = storage.connection();
    let mut deleted = Vec::new();
    let mut failed = Vec::new();

    let mut clear_table = |table: &str| {
        let sql = format!("DELETE FROM {table}");
        match connection.with_connection(|conn| Ok(conn.execute_batch(&sql)?)) {
            Ok(()) => deleted.push(table.to_string()),
            Err(error) => failed.push(format!("{table}: {error}")),
        }
    };

    for table in REQUIRED_LOCAL_TABLES {
        clear_table(table);
    }
    if scope.query_history {
        clear_table("sql_execution_history");
    }
    if scope.command_history {
        clear_table("terminal_command_history");
        clear_table("quick_commands");
    }

    // 密钥相关文件：主密钥本体、验证数据与版本信息。
    // 顺序要紧：先清 pending 标记再删元信息文件，否则删完之后再写标记会凭空
    // 生成一个版本为 1 的元信息文件，重置就不算"等同全新安装"了。
    let _ = crypto::reset_repo_password();
    master_key_meta::set_pending_cloud_reencrypt(false);
    master_key_meta::set_pending_cloud_wipe(false);
    master_key_meta::clear();
    AppSettings::update_and_save(cx, |settings| {
        settings.personal_sync.webdav.password = String::new();
    });

    let mut outcome = ResetOutcome { deleted, failed };
    if scope.notes {
        clear_notes(&mut outcome);
    }
    if let Some(hooks) = host_hooks() {
        (hooks.wipe_cloud)(scope.cloud_wipe_scope(), cx);
    }

    Ok(outcome)
}

fn clear_notes(outcome: &mut ResetOutcome) {
    let Some(root) = host_hooks().and_then(|hooks| (hooks.notes_root)()) else {
        return;
    };
    if !root.exists() {
        outcome.deleted.push("notes".to_string());
        return;
    }
    match std::fs::remove_dir_all(&root) {
        Ok(()) => outcome.deleted.push("notes".to_string()),
        Err(error) => outcome.failed.push(format!("notes: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scope_deletes_everything() {
        let scope = ResetScope::default();
        assert!(scope.team_data);
        assert!(scope.query_history);
        assert!(scope.command_history);
        assert!(scope.notes);
        assert!(scope.cloud_sync);
    }

    #[test]
    fn cloud_wipe_scope_follows_scope_fields() {
        let scope = ResetScope {
            team_data: false,
            query_history: true,
            command_history: true,
            notes: false,
            cloud_sync: true,
        };
        assert_eq!(
            scope.cloud_wipe_scope(),
            CloudWipeScope {
                personal_sync: true,
                team_data: false,
            }
        );
    }
}

/// 重置范围的一行：开关打开 = 删除该项。顺序与 [`ResetScope`] 字段一致。
pub(super) const RESET_SCOPE_ROWS: &[(&str, &str, &str)] = &[
    (
        "reset-scope-team-data",
        "云端团队数据",
        "云端共享的团队连接数据（由团队密钥加密）",
    ),
    (
        "reset-scope-query-history",
        "查询历史",
        "保存在本机的 SQL 执行历史",
    ),
    (
        "reset-scope-command-history",
        "命令历史与快捷命令",
        "终端命令历史与已保存的快捷命令",
    ),
    (
        "reset-scope-notes",
        "笔记文件",
        "笔记目录中的 Markdown 文件（从磁盘删除）",
    ),
    (
        "reset-scope-cloud-sync",
        "云同步数据与版本信息",
        "Navop Cloud 或已配置同步目标中的同步记录与版本信息",
    ),
];

/// 读取 / 写入勾选状态（下标与 [`RESET_SCOPE_ROWS`] 对应）。
pub(super) fn scope_toggle(scope: &mut ResetScope, index: usize, checked: Option<bool>) -> bool {
    if let Some(checked) = checked {
        match index {
            0 => scope.team_data = checked,
            1 => scope.query_history = checked,
            2 => scope.command_history = checked,
            3 => scope.notes = checked,
            _ => scope.cloud_sync = checked,
        }
    }
    match index {
        0 => scope.team_data,
        1 => scope.query_history,
        2 => scope.command_history,
        3 => scope.notes,
        _ => scope.cloud_sync,
    }
}

/// 重置范围的一行：开关自带内部状态，点击后自己重绘，不需要手动刷新。
pub(super) fn scope_row(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    checked: bool,
    index: usize,
    cx: &App,
) -> gpui::Div {
    h_flex()
        .justify_between()
        .items_start()
        .gap_3()
        .py_1()
        .child(
            v_flex()
                .gap_1()
                .flex_1()
                .child(div().text_sm().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(
            Switch::new(id)
                .checked(checked)
                .on_click(move |value, _window, cx| {
                    let mut scope = reset_scope(cx);
                    scope_toggle(&mut scope, index, Some(*value));
                    set_reset_scope(scope, cx);
                }),
        )
}
