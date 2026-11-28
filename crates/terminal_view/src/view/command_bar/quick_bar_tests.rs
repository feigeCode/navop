use super::*;
use crate::view::command_bar::render::popover_offset_for;

fn command(raw: &str) -> QuickCommand {
    QuickCommand::new(raw.to_string())
}

fn named(command: QuickCommand, name: &str) -> QuickCommand {
    let mut command = command;
    command.name = Some(name.to_string());
    command
}

#[test]
fn pinned_quick_commands_keeps_only_pinned_in_existing_order() {
    let commands = vec![
        command("cd app").pinned(),
        command("kubectl get pods"),
        command("tail -f app.log").pinned(),
    ];

    let pinned = pinned_quick_commands(&commands)
        .into_iter()
        .map(|command| command.command.as_str())
        .collect::<Vec<_>>();

    assert_eq!(vec!["cd app", "tail -f app.log"], pinned);
}

fn grouped(mut command: QuickCommand, group: &str) -> QuickCommand {
    command.group_name = Some(group.to_string());
    command
}

#[test]
fn quick_bar_groups_put_pinned_commands_first_inside_each_group() {
    let commands = vec![
        grouped(command("cd app"), "部署"),
        grouped(command("kubectl get pods").pinned(), "部署"),
        grouped(command("docker ps"), "运维"),
    ];

    let groups = quick_bar_groups(&commands);
    let names = groups
        .iter()
        .map(|group| group.name.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(vec!["部署", "运维"], names);

    let deploy = groups[0]
        .commands
        .iter()
        .map(|command| command.command.as_str())
        .collect::<Vec<_>>();
    assert_eq!(vec!["kubectl get pods", "cd app"], deploy);
}

#[test]
fn quick_bar_groups_keep_ungrouped_commands_in_the_same_row() {
    let commands = vec![command("ls"), grouped(command("cd app"), "部署")];

    let groups = quick_bar_groups(&commands);
    assert_eq!(2, groups.len());
    assert!(groups[0].name.is_none(), "未分组指令要保持无标题分组");
    assert_eq!(Some("部署".to_string()), groups[1].name);
}

#[test]
fn named_group_detection_ignores_blank_names() {
    assert!(!has_named_quick_group(&[grouped(command("ls"), "   ")]));
    assert!(has_named_quick_group(&[grouped(command("ls"), " 部署 ")]));
    assert!(!has_named_quick_group(&[command("ls")]));
}

#[test]
fn group_label_is_trimmed_and_blank_groups_have_none() {
    let groups = quick_bar_groups(&[grouped(command("a"), " 部署 "), command("b")]);
    assert_eq!(Some("部署".to_string()), quick_bar_group_label(&groups[0]));
    assert_eq!(None, quick_bar_group_label(&groups[1]));
}

#[test]
fn chip_label_prefers_trimmed_name() {
    let command = named(
        command("kubectl rollout status deployment/app"),
        "  Deploy  ",
    );
    assert_eq!("Deploy", quick_command_chip_label(&command));
}

#[test]
fn chip_label_falls_back_to_first_command_line() {
    let command = command("  cd app  \nls -la");
    assert_eq!("cd app", quick_command_chip_label(&command));
}

#[test]
fn chip_label_ignores_blank_name() {
    let command = named(command("cd job-client"), "   ");
    assert_eq!("cd job-client", quick_command_chip_label(&command));
}

#[test]
fn popover_offset_only_reserves_command_bar_body() {
    // 折叠行本身就是快捷指令栏，不再额外占一行高度。
    assert_eq!(30.0 + 8.0, popover_offset_for(true, 80.0));
    assert_eq!(120.0 + 8.0, popover_offset_for(false, 120.0));
}

#[test]
fn quick_bar_reuses_shared_execution_path_and_scrolls_horizontally() {
    let source = include_str!("quick_bar.rs");
    assert!(
        source.contains(".overflow_x_scroll()"),
        "置顶横条必须支持横向滚动"
    );
    assert!(
        source.contains("this.choose_command(value.clone(), window, cx)"),
        "点击 chip 必须复用 choose_command，保证参数与自动执行行为一致"
    );

    let render = include_str!("render.rs");
    assert!(
        render.contains("render_quick_command_chips"),
        "折叠行必须直接挂载置顶 chip 横条"
    );
    assert!(
        render.contains("render_quick_command_icon_button"),
        "快捷指令入口必须固定在折叠行右侧"
    );
}

#[test]
fn quick_bar_hint_explains_where_pinned_commands_go() {
    let source = include_str!("quick_bar.rs");
    assert!(
        source.contains("quick_bar_hint"),
        "没有置顶指令时需要提示置顶后会显示在这里"
    );
    assert!(
        source.contains("render_quick_bar_hint"),
        "空状态提示要独立成段，便于后续复用与测试"
    );
}
