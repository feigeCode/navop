//! 快捷指令参数占位符。
//!
//! 快捷指令可以用 `{{name}}` 表示运行前需要填写的参数。参数解析必须由侧边栏、
//! 命令栏和快捷键共用，否则同一个快捷指令会因入口不同而把占位符原样发到终端。

use std::rc::Rc;

use gpui::{App, AppContext as _, Hsla, ParentElement, Styled, Window, div};
use gpui_component::{
    WindowExt,
    input::{Input, InputState},
    v_flex,
};
use rust_i18n::t;

/// 提取 `{{name}}` 形式的参数，按首次出现顺序去重。
pub(crate) fn quick_command_parameters(command: &str) -> Vec<String> {
    let mut parameters = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let name = after_start[..end].trim();
        if !name.is_empty() && !parameters.iter().any(|parameter| parameter == name) {
            parameters.push(name.to_string());
        }
        rest = &after_start[end + 2..];
    }
    parameters
}

pub(crate) fn replace_quick_command_parameters(
    command: &str,
    parameters: &[String],
    values: &[String],
) -> String {
    let mut rendered = String::with_capacity(command.len());
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            rendered.push_str(&rest[start..]);
            return rendered;
        };
        let name = after_start[..end].trim();
        match parameters
            .iter()
            .position(|parameter| parameter == name)
            .and_then(|index| values.get(index))
        {
            Some(value) => rendered.push_str(value),
            None => {
                rendered.push_str("{{");
                rendered.push_str(&after_start[..end]);
                rendered.push_str("}}");
            }
        }
        rest = &after_start[end + 2..];
    }
    rendered.push_str(rest);
    rendered
}

/// 有参数时打开填写对话框，并在确认后把替换完成的命令交给调用方。
///
/// 返回 `false` 表示命令不含参数，调用方应继续原来的执行路径。
pub(crate) fn prompt_for_quick_command_parameters(
    command: &str,
    hint_color: Hsla,
    window: &mut Window,
    cx: &mut App,
    on_submit: impl Fn(String, &mut Window, &mut App) + 'static,
) -> bool {
    let parameters = quick_command_parameters(command);
    if parameters.is_empty() {
        return false;
    }

    let inputs = parameters
        .iter()
        .map(|parameter| {
            let placeholder =
                t!("QuickCommand.parameter_placeholder", name = parameter).to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        })
        .collect::<Vec<_>>();
    let inputs_for_ok = inputs.clone();
    let parameters_for_ok = parameters.clone();
    let command = command.to_string();
    let on_submit: Rc<dyn Fn(String, &mut Window, &mut App)> = Rc::new(on_submit);

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let inputs = inputs.clone();
        let inputs_for_ok = inputs_for_ok.clone();
        let parameters = parameters_for_ok.clone();
        let parameters_for_ok = parameters.clone();
        let command = command.clone();
        let on_submit = on_submit.clone();
        dialog
            .title(t!("QuickCommand.parameters_title").to_string())
            .width(gpui::px(440.0))
            .confirm()
            .on_ok(move |_, window, cx| {
                let values = inputs_for_ok
                    .iter()
                    .map(|input| input.read(cx).value().to_string())
                    .collect::<Vec<_>>();
                let command =
                    replace_quick_command_parameters(&command, &parameters_for_ok, &values);
                on_submit(command, window, cx);
                true
            })
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(hint_color)
                            .child(t!("QuickCommand.parameters_hint").to_string()),
                    )
                    .children(
                        parameters
                            .iter()
                            .zip(inputs.iter())
                            .map(|(parameter, input)| {
                                v_flex()
                                    .gap_1()
                                    .child(div().text_sm().child(parameter.clone()))
                                    .child(Input::new(input).w_full())
                            }),
                    ),
            )
    });

    true
}

#[cfg(test)]
mod tests {
    use super::{quick_command_parameters, replace_quick_command_parameters};

    #[test]
    fn parameterized_quick_commands_extract_unique_names_and_replace_values() {
        let command = "ssh {{ user }}@{{host}} && echo {{user}} {{missing";
        let parameters = quick_command_parameters(command);
        assert_eq!(vec!["user".to_string(), "host".to_string()], parameters);

        let rendered = replace_quick_command_parameters(
            command,
            &parameters,
            &["root".to_string(), "example.com".to_string()],
        );
        assert_eq!("ssh root@example.com && echo root {{missing", rendered);
    }

    #[test]
    fn parameter_replacement_preserves_the_auto_run_marker() {
        let command = "deploy {{target}}\n";
        let parameters = quick_command_parameters(command);
        let rendered =
            replace_quick_command_parameters(command, &parameters, &["production".to_string()]);

        assert_eq!("deploy production\n", rendered);
    }
}
