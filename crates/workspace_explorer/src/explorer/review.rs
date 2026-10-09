//! 从整轮快照 patch 里裁出「某一个文件」那一段。
//!
//! Review 面板展示的是整轮 checkpoint 的 diff——一份多文件 patch。而聊天里的
//! 「在 Review 中打开」指向的是单个文件，所以要把整份 patch 收敛到那一个文件。
//!
//! 这里是**按段裁**，不是重算 diff：快照已经落定，重算只会得到另一份「看起来
//! 差不多」的差异，同一轮改动在两处显示成不完全一样的 diff 比不显示更糟。

use gpui_component::diff::DiffFile;
use std::path::Path;

/// 从整轮 patch 里裁出 `path` 这个文件的那一段。
///
/// 只按行首的 `diff --git ` 分段：unified diff 的正文行一律带 ` ` / `+` / `-` /
/// `\` 前缀，裸的 `diff --git ` 只能出现在段首，所以 hunk 内容里恰好有同样文字
/// 也不会把这个文件切成两段。没有 Git 头的纯 unified patch 整份算一段。
pub(super) fn patch_section_for_path<'a>(patch: &'a str, path: &str) -> Option<&'a str> {
    let mut starts = vec![0usize];
    let mut offset = 0usize;
    for line in patch.split_inclusive('\n') {
        // 第 0 行前面没有「上一段」，只有从第 1 行起的 `diff --git ` 才是段界。
        if offset > 0 && line.starts_with("diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    starts.push(patch.len());
    starts
        .windows(2)
        .map(|window| &patch[window[0]..window[1]])
        .find(|section| section_describes(section, path))
}

/// 这一段 patch 描述的是不是 `path` 这个文件。
///
/// 改名/复制时新旧两侧都算命中：用户点的是「这个文件这轮变了」，不是「这个路径
/// 这轮出现了」。解析不出片段就当作不匹配——宁可退到下一档来源，也不要拿一段
/// 读不懂的 patch 去开一个空 diff 页。
fn section_describes(section: &str, path: &str) -> bool {
    let Ok(files) = DiffFile::parse(section) else {
        return false;
    };
    files.iter().any(|file| {
        [
            Some(file.path()),
            file.original_path(),
            file.modified_path(),
        ]
        .into_iter()
        .flatten()
        .any(|candidate| same_path(candidate, path))
    })
}

/// 路径相等判定：两侧都归一成 `/` 分隔、无 `./` 前缀再比。
///
/// 不剥 `a/` `b/` 前缀：`DiffFile::path()` 已经由解析器剥过，而请求侧的路径从来
/// 不带这层前缀；多剥一层会把仓库里真实的 `a/x.rs` 错认成 `x.rs`。
pub(super) fn same_path(left: &str, right: &str) -> bool {
    normalize_path(left) == normalize_path(right)
}

/// 把请求侧路径收敛成仓库相对的 `/` 分隔形式；不在工作区里就原样归一。
pub(super) fn relative_to_root(path: &Path, root: &Path) -> String {
    normalize_path(&path.strip_prefix(root).unwrap_or(path).to_string_lossy())
}

/// 「这个路径相对哪个根记的」的全部候选，根的顺序即优先级，重复的去掉。
///
/// 卡片给的是绝对路径，而我们不知道它是相对工作区根还是相对仓库根记的——工作区是
/// 仓库子目录时这两个根不同（`/repo/crates/db` 与 `/repo`）。逐条试比只认一个根
/// 更省事也更不容易错：路径对不上时的后果是静默退到下一档来源。
pub(super) fn relative_candidates(path: &Path, roots: &[&Path]) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for root in roots {
        let relative = relative_to_root(path, root);
        if !candidates.contains(&relative) {
            candidates.push(relative);
        }
    }
    candidates
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests;
