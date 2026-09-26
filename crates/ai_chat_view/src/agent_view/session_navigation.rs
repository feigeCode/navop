//! 会话间「后退 / 前进」导航栈。
//!
//! 纯状态机，不接触 GPUI / Runtime：谁在什么时候调它由 `agent_view` 决定，
//! 这里只保证栈本身的正确性。会话 id 用字符串（与侧栏 / Runtime 的 uid 同源）。

/// 会话导航栈。
///
/// - [`SessionNavigation::visit`]：普通切换（点侧栏行、切换器提交），把当前会话
///   压入 back 并清空 forward——走新分支就该截断旧的前进路径（浏览器同款语义）。
/// - [`SessionNavigation::go_back`] / [`SessionNavigation::go_forward`]：在两个栈
///   之间移动，调用方拿到目标后自行完成切换。
/// - [`SessionNavigation::remove`]：会话被删除 / 归档时清掉它的所有痕迹，
///   否则后退会落在一个已经不存在的会话上。
#[derive(Default)]
pub(super) struct SessionNavigation {
    back: Vec<String>,
    forward: Vec<String>,
}

impl SessionNavigation {
    /// 普通访问 `next`：当前会话压入 back，forward 清空。
    ///
    /// `current == next` 或没有当前会话时不产生历史（重复点击同一行不该
    /// 把 back 栈垫满同一个 id）。
    pub(super) fn visit(&mut self, current: Option<String>, next: &str) {
        if let Some(current) = current.filter(|current| current != next) {
            self.back.push(current);
            self.forward.clear();
        }
    }

    /// 下一个后退目标（只看不取）。
    pub(super) fn back_target(&self) -> Option<&str> {
        self.back.last().map(String::as_str)
    }

    /// 下一个前进目标（只看不取）。
    pub(super) fn forward_target(&self) -> Option<&str> {
        self.forward.last().map(String::as_str)
    }

    /// 执行后退：back 栈顶弹出、当前会话压入 forward，返回目标。
    ///
    /// **先移动栈再返回**：即使调用方随后切换失败，栈的状态也是自洽的
    /// （这条路径丢了就是丢了，比「后退永远回不到同一条」更可预测）。
    pub(super) fn go_back(&mut self, current: &str) -> Option<String> {
        let target = self.back.pop()?;
        self.forward.push(current.to_string());
        Some(target)
    }

    /// 执行前进：forward 栈顶弹出、当前会话压入 back，返回目标。
    pub(super) fn go_forward(&mut self, current: &str) -> Option<String> {
        let target = self.forward.pop()?;
        self.back.push(current.to_string());
        Some(target)
    }

    /// 会话消失（删除 / 归档）时移除它在两个栈里的痕迹。
    pub(super) fn remove(&mut self, uid: &str) {
        self.back.retain(|entry| entry != uid);
        self.forward.retain(|entry| entry != uid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visit_pushes_current_and_truncates_forward() {
        let mut nav = SessionNavigation::default();
        nav.visit(Some("a".into()), "b");
        nav.visit(Some("b".into()), "c");
        assert_eq!(Some("b"), nav.back_target());
        assert_eq!(None, nav.forward_target(), "visit 必须清空 forward");

        // 走过一段后退后再 visit：旧 forward 分支被截断。
        let target = nav.go_back("c").expect("back target exists");
        assert_eq!("b", target);
        nav.visit(Some("b".into()), "d");
        assert_eq!(None, nav.forward_target(), "新分支截断旧前进路径");
    }

    #[test]
    fn back_and_forward_round_trip() {
        let mut nav = SessionNavigation::default();
        nav.visit(Some("a".into()), "b");
        nav.visit(Some("b".into()), "c");

        assert_eq!(Some("b".to_string()), nav.back_target().map(str::to_string));

        let back = nav.go_back("c").expect("go back");
        assert_eq!("b", back);
        assert_eq!(
            Some("c".to_string()),
            nav.forward_target().map(str::to_string)
        );
        assert_eq!(Some("a".to_string()), nav.back_target().map(str::to_string));

        let forward = nav.go_forward("b").expect("go forward");
        assert_eq!("c", forward);
        assert_eq!(None, nav.forward_target());
        assert_eq!(Some("b".to_string()), nav.back_target().map(str::to_string));
    }

    #[test]
    fn visiting_the_same_session_records_nothing() {
        let mut nav = SessionNavigation::default();
        nav.visit(Some("a".into()), "a");
        assert_eq!(None, nav.back_target());

        nav.visit(None, "a");
        assert_eq!(None, nav.back_target(), "没有当前会话时不产生历史");
    }

    #[test]
    fn remove_erases_the_session_from_both_stacks() {
        let mut nav = SessionNavigation::default();
        nav.visit(Some("a".into()), "b");
        nav.visit(Some("b".into()), "c");
        let _ = nav.go_back("c"); // back: [a], forward: [c]

        nav.remove("a");
        assert_eq!(None, nav.back_target());
        assert_eq!(
            Some("c".to_string()),
            nav.forward_target().map(str::to_string)
        );

        nav.remove("c");
        assert_eq!(None, nav.forward_target());
    }

    #[test]
    fn go_back_on_empty_stack_returns_none_and_keeps_state() {
        let mut nav = SessionNavigation::default();
        assert!(nav.go_back("a").is_none());
        assert!(nav.go_forward("a").is_none());
        assert_eq!(None, nav.back_target());
    }
}
