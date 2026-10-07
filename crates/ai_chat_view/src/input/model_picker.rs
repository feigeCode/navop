//! 模型选择器的分组与列表项投影。
//!
//! 分组逻辑是纯函数，便于在不依赖 GPUI 的单元测试里锁住「同一 provider 的模型
//! 聚在一起、且组顺序按首次出现顺序」这一行为。
//!
//! [`ModelChoice`] 把 [`ComposerModelOption`] 投影成组件库 `Select` 认识的列表项：
//! 主标题是 `provider / model`，次要说明（`hint`）另起一行 —— 后者常常是模型的
//! 人类可读名（ACP agent 会给），挤进主标题会变成一长串。

use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::ActiveTheme;
use gpui_component::IndexPath;
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{SelectGroup, SelectItem};
use gpui_component::v_flex;

use super::context::{ComposerModel, ComposerModelOption};

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

/// `Select` 的列表项。
#[derive(Clone, Debug)]
pub(crate) struct ModelChoice {
    option: ComposerModelOption,
}

impl ModelChoice {
    pub(crate) fn new(option: ComposerModelOption) -> Self {
        Self { option }
    }
}

impl SelectItem for ModelChoice {
    /// 直接把整个选项当值：选中回调拿到的就是完整信息，不用再按 id 反查。
    type Value = ComposerModelOption;

    fn title(&self) -> SharedString {
        self.option.display_label()
    }

    fn value(&self) -> &Self::Value {
        &self.option
    }

    /// 搜索面必须覆盖 `hint`：ACP agent 给的模型名（人类可读）只出现在那里，
    /// 只按 `provider / model` 匹配的话，用户照屏幕上看到的名字搜反而搜不到。
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let option = &self.option;
        option.model.to_lowercase().contains(&query)
            || option.provider_label.to_lowercase().contains(&query)
            || option.id.to_lowercase().contains(&query)
            || option
                .hint
                .as_ref()
                .is_some_and(|hint| hint.to_lowercase().contains(&query))
    }

    fn render(&self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut column = v_flex()
            .gap(px(1.0))
            // 弹层自己的宽度没有调试选择器 —— 它由组件库 `select` 内部渲染,拿不到挂点;
            // 行宽又是按内容走的,量不到弹层。但行的**位置**在弹层里,所以这个选择器是
            // 「弹层在哪」唯一可观测的缝:`agent_input` 的
            // `model_menu_popup_is_wider_than_the_squeezed_trigger` 靠它确认点开后弹层
            // 真的比触发器宽(被视口右缘往回压),而没有退回 `Length::Auto`。
            .debug_selector(|| "agent-model-option".to_string())
            .child(div().text_sm().child(self.option.display_label()));
        if let Some(hint) = &self.option.hint {
            column = column.child(div().text_xs().text_color(muted).child(hint.clone()));
        }
        column
    }
}

/// 把选项投成组件库 `Select` 的输入：按 provider 分组的可搜索列表。
pub(crate) fn model_groups(
    options: &[ComposerModelOption],
) -> SearchableVec<SelectGroup<ModelChoice>> {
    SearchableVec::new(
        group_models(options)
            .into_iter()
            .map(|group| {
                SelectGroup::new(group.provider_label)
                    .items(group.options.into_iter().map(ModelChoice::new))
            })
            .collect::<Vec<_>>(),
    )
}

/// 在分组列表里反查「当前模型」的位置。
///
/// 注入的上下文只给 `(provider, model)`，而 `Select` 认的是下标；两者对不上时
/// 返回 `None`（宁可显示空态，也不假装选中了别的模型）。
pub(crate) fn selected_model_index(
    options: &[ComposerModelOption],
    current: Option<&ComposerModel>,
) -> Option<IndexPath> {
    let current = current?;
    group_models(options)
        .iter()
        .enumerate()
        .find_map(|(section, group)| {
            group
                .options
                .iter()
                .position(|option| {
                    option.provider_label == current.provider && option.model == current.model
                })
                .map(|row| IndexPath::default().section(section).row(row))
        })
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

    fn choice(provider: &str, model: &str, hint: Option<&str>) -> ModelChoice {
        let option = match hint {
            Some(hint) => option(provider, model).with_hint(hint),
            None => option(provider, model),
        };
        ModelChoice::new(option)
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

    #[test]
    fn search_matches_the_hint_not_only_the_provider_slash_model_title() {
        let claude = choice("opencode", "claude-sonnet-4-5", Some("Claude Sonnet 4.5"));

        // 屏幕上显示的是 hint，用户就会照它搜。
        assert!(claude.matches("Sonnet"), "hint 必须参与匹配");
        assert!(claude.matches("claude sonnet"));
        // 主标题与 id 也照样能搜。
        assert!(claude.matches("opencode"));
        assert!(claude.matches("4-5"));
        // 大小写不敏感。
        assert!(claude.matches("CLAUDE"));
        // 空查询 = 全部命中（清空搜索框后要能看到完整列表）。
        assert!(claude.matches("   "));
        assert!(!claude.matches("gpt"));
    }

    #[test]
    fn selected_index_points_at_the_group_and_row_of_the_current_model() {
        let options = vec![
            option("OpenAI", "gpt-4.1"),
            option("Anthropic", "claude-sonnet"),
            option("OpenAI", "gpt-4.1-mini"),
        ];

        // 分组后 OpenAI 是第 0 组（两个），Anthropic 是第 1 组。
        let current = ComposerModel::new("Anthropic", "claude-sonnet");
        assert_eq!(
            Some(IndexPath::default().section(1).row(0)),
            selected_model_index(&options, Some(&current))
        );

        let current = ComposerModel::new("OpenAI", "gpt-4.1-mini");
        assert_eq!(
            Some(IndexPath::default().section(0).row(1)),
            selected_model_index(&options, Some(&current))
        );
    }

    #[test]
    fn selected_index_is_none_when_the_context_model_is_not_offered() {
        let options = vec![option("OpenAI", "gpt-4.1")];

        // 探测失败 / 换过 agent 时上下文里可能留着一个已下架的模型名：
        // 这时宁可不选中，也不能顺手选中列表里的第一个。
        let stale = ComposerModel::new("OpenAI", "gpt-5");
        assert_eq!(None, selected_model_index(&options, Some(&stale)));
        assert_eq!(None, selected_model_index(&options, None));
    }
}
