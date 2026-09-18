//! 模型选择器的分组逻辑。
//!
//! 纯函数，便于在不依赖 GPUI 的单元测试里锁住「同一 provider 的模型聚在一起、
//! 且组顺序按首次出现顺序」这一行为。

use gpui::SharedString;

use super::context::ComposerModelOption;

/// 同一 provider 下的模型分组。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelGroup {
    pub(crate) provider_label: SharedString,
    pub(crate) options: Vec<ComposerModelOption>,
}

/// 按 provider 分组，provider 组顺序取其在 `options` 中首次出现的顺序。
pub(crate) fn group_models(options: &[ComposerModelOption]) -> Vec<ModelGroup> {
    let mut groups: Vec<ModelGroup> = Vec::new();
    for option in options {
        match groups
            .iter_mut()
            .find(|group| group.provider_label == option.provider_label)
        {
            Some(group) => group.options.push(option.clone()),
            None => groups.push(ModelGroup {
                provider_label: option.provider_label.clone(),
                options: vec![option.clone()],
            }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(provider: &str, model: &str) -> ComposerModelOption {
        ComposerModelOption::new(
            format!("{provider}-{model}"),
            provider,
            provider,
            model,
        )
    }

    #[test]
    fn groups_same_provider_and_keeps_first_appearance_order() {
        let options = vec![
            option("OpenAI", "gpt-4.1"),
            option("Anthropic", "claude-sonnet"),
            option("OpenAI", "gpt-4.1-mini"),
        ];

        let groups = group_models(&options);

        assert_eq!(2, groups.len());
        assert_eq!("OpenAI", groups[0].provider_label.as_ref());
        assert_eq!("Anthropic", groups[1].provider_label.as_ref());
        assert_eq!(2, groups[0].options.len());
        assert_eq!("gpt-4.1-mini", groups[0].options[1].model.as_ref());
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(group_models(&[]).is_empty());
    }

    #[test]
    fn single_provider_keeps_all_options_in_order() {
        let options = vec![option("Ollama", "llama3"), option("Ollama", "qwen3")];

        let groups = group_models(&options);

        assert_eq!(1, groups.len());
        let models: Vec<&str> = groups[0]
            .options
            .iter()
            .map(|option| option.model.as_ref())
            .collect();
        assert_eq!(vec!["llama3", "qwen3"], models);
    }
}
