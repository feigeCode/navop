//! 文件打开契约：领域可以接管自己认识的文件类型（例如终端录制）。

use gpui::{App, Window};
use std::path::Path;

/// 文件打开处理器。
///
/// 只按路径做轻量判断（扩展名等），真正的加载可以在 `open` 内部异步完成。
pub trait FileOpenHandler: Send + Sync {
    /// 是否接管该路径。
    fn handles(&self, path: &Path) -> bool;

    /// 打开文件；实现方自行处理错误提示。
    fn open(&self, path: &Path, window: &mut Window, cx: &mut App);
}
