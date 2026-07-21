use super::WorkspaceExplorer;
use gpui::{Context, EventEmitter};

/// 面板在宿主侧边栏中的停靠位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerFramePlacement {
    Left,
    Right,
    Bottom,
}

/// 工作区浏览器向宿主发出的框架事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceExplorerEvent {
    Close,
    MoveTo(ExplorerFramePlacement),
}

impl EventEmitter<WorkspaceExplorerEvent> for WorkspaceExplorer {}

impl WorkspaceExplorer {
    pub fn set_frame_placement(
        &mut self,
        placement: ExplorerFramePlacement,
        cx: &mut Context<Self>,
    ) {
        if self.frame_placement == placement {
            return;
        }
        self.frame_placement = placement;
        cx.notify();
    }

    /// 切换点文件可见性，并按新过滤规则重建列表。
    pub fn toggle_show_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.rebuild_file_tree(cx);
    }

    /// 切换 Git ignored 文件可见性，并按新过滤规则重建列表。
    pub fn toggle_show_ignored(&mut self, cx: &mut Context<Self>) {
        self.show_ignored = !self.show_ignored;
        self.rebuild_file_tree(cx);
    }

    fn rebuild_file_tree(&mut self, cx: &mut Context<Self>) {
        self.listings.clear();
        self.expanded.clear();
        self.loading_directories.clear();
        self.selected_path = None;
        self.refresh(cx);
    }
}
