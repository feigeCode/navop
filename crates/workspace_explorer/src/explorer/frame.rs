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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceExplorerEvent {
    Close,
    MoveTo(ExplorerFramePlacement),
    SyncTerminalCwd,
    RootChanged(std::path::PathBuf),
    /// 请求宿主用已配置的 LLM 生成提交信息（Explorer 不依赖 LLM 层）。
    CommitMessageRequested,
    /// 用户请求查看一个文档（点开文件或 Git 变更）。宿主工作台借此把
    /// 审阅面板带到前台——Explorer 只知道编辑器实体，不知道落位布局。
    DocumentRequested,
    /// 某个会话「可以回滚到的轮次」列表发生变化：快照刚锚定成功，或回滚后把
    /// 后续轮次截掉了。
    ///
    /// 这里送**全量**列表而不是增量，因为快照由 Explorer 独占：宿主只需要把看到的
    /// 结果原样灌进视图，不用自己维护一份可能漂移的镜像。
    RestorableTurnsChanged {
        session_id: String,
        turn_ids: Vec<String>,
    },
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
