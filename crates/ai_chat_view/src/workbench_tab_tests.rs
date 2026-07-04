use crate::{CHAT_TAB_CONTENT_KEY, CHAT_TASK_SIDEBAR_TITLE, ChatTabContent};
use one_core::tab_container::TabContent;

fn assert_tab_content<T: TabContent>() {}

#[test]
fn chat_tab_content_is_a_tab_container_content() {
    assert_tab_content::<ChatTabContent>();
    assert_eq!("ChatWorkbench", CHAT_TAB_CONTENT_KEY);
}

#[test]
fn chat_sidebar_uses_task_language() {
    assert_eq!("历史任务", CHAT_TASK_SIDEBAR_TITLE);
}
