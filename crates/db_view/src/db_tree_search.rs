//! 数据库树搜索框的匹配规则。
//!
//! 对齐 IDEA 搜索框的三个开关：
//! - `Cc` 区分大小写
//! - `W` 全词匹配（下划线、点、连字符与驼峰都算词边界）
//! - `.*` 正则表达式（开启时忽略全词开关）

use regex::{Regex, RegexBuilder};
use rust_i18n::t;

/// 搜索框右侧的匹配开关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeSearchToggle {
    /// 区分大小写。
    MatchCase,
    /// 全词匹配。
    WholeWord,
    /// 正则表达式。
    Regex,
}

impl TreeSearchToggle {
    /// 三个开关，按显示顺序排列。
    pub(crate) const ALL: [Self; 3] = [Self::MatchCase, Self::WholeWord, Self::Regex];

    /// 开关上的文字（与 IDEA 一致）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MatchCase => "Cc",
            Self::WholeWord => "W",
            Self::Regex => ".*",
        }
    }

    /// 元素 id。
    pub(crate) fn element_id(self) -> &'static str {
        match self {
            Self::MatchCase => "tree-search-match-case",
            Self::WholeWord => "tree-search-whole-word",
            Self::Regex => "tree-search-regex",
        }
    }

    /// 悬停提示；正则写错时正则开关直接说明原因。
    pub(crate) fn tooltip(self, invalid_regex: bool) -> String {
        match self {
            Self::MatchCase => t!("DbTreeView.search_match_case").to_string(),
            Self::WholeWord => t!("DbTreeView.search_whole_word").to_string(),
            Self::Regex if invalid_regex => t!("DbTreeView.search_regex_invalid").to_string(),
            Self::Regex => t!("DbTreeView.search_regex").to_string(),
        }
    }
}

/// 树搜索的匹配方式（对应搜索框右侧的三个开关）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TreeSearchOptions {
    /// 区分大小写。
    pub(crate) match_case: bool,
    /// 全词匹配。
    pub(crate) whole_word: bool,
    /// 正则表达式。
    pub(crate) regex: bool,
}

impl TreeSearchOptions {
    /// 某个开关当前是否打开。
    pub(crate) fn is_on(self, toggle: TreeSearchToggle) -> bool {
        match toggle {
            TreeSearchToggle::MatchCase => self.match_case,
            TreeSearchToggle::WholeWord => self.whole_word,
            TreeSearchToggle::Regex => self.regex,
        }
    }

    /// 切换某个开关之后的选项。
    pub(crate) fn toggled(self, toggle: TreeSearchToggle) -> Self {
        let mut next = self;
        match toggle {
            TreeSearchToggle::MatchCase => next.match_case = !self.match_case,
            TreeSearchToggle::WholeWord => next.whole_word = !self.whole_word,
            TreeSearchToggle::Regex => next.regex = !self.regex,
        }
        next
    }

    /// 该开关是否因为其它开关而不可用（正则模式下「全词」没有意义）。
    pub(crate) fn is_disabled(self, toggle: TreeSearchToggle) -> bool {
        toggle == TreeSearchToggle::WholeWord && self.regex
    }

    /// 搜索词是否写成了非法正则。
    pub(crate) fn has_invalid_regex(self, query: &str) -> bool {
        self.regex && TreeSearchMatcher::new(query, self).is_invalid_regex()
    }
}

/// 一个搜索开关在界面上的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeSearchToggleView {
    /// 开关上的文字。
    pub(crate) label: &'static str,
    /// 元素 id。
    pub(crate) element_id: &'static str,
    /// 是否处于打开状态。
    pub(crate) checked: bool,
    /// 是否不可用（正则模式下「全词」没有意义）。
    pub(crate) disabled: bool,
    /// 当前设置本身有问题（只有正则开关会）。
    pub(crate) invalid: bool,
    /// 悬停提示。
    pub(crate) tooltip: String,
    /// 点击后应当生效的选项。
    pub(crate) next: TreeSearchOptions,
}

/// 搜索框右侧三个开关的界面描述，按显示顺序排列。
pub(crate) fn search_toggle_views(
    options: TreeSearchOptions,
    invalid_regex: bool,
) -> [TreeSearchToggleView; TreeSearchToggle::ALL.len()] {
    TreeSearchToggle::ALL.map(|toggle| TreeSearchToggleView {
        label: toggle.label(),
        element_id: toggle.element_id(),
        checked: options.is_on(toggle),
        disabled: options.is_disabled(toggle),
        invalid: invalid_regex && toggle == TreeSearchToggle::Regex,
        tooltip: toggle.tooltip(invalid_regex),
        next: options.toggled(toggle),
    })
}

/// 一次搜索用到的匹配器：查询词与开关只编译一次，整棵树复用。
pub(crate) struct TreeSearchMatcher {
    options: TreeSearchOptions,
    /// 非正则模式下待匹配的内容：区分大小写时保留原样，否则小写。
    needle: Option<String>,
    /// 正则模式下已编译的表达式。
    pattern: Option<Regex>,
    /// 正则模式下的搜索词非法，一律视为没有命中。
    invalid_regex: bool,
}

impl TreeSearchMatcher {
    pub(crate) fn new(query: &str, options: TreeSearchOptions) -> Self {
        let query = query.trim();
        if query.is_empty() {
            return Self {
                options,
                needle: None,
                pattern: None,
                invalid_regex: false,
            };
        }

        if options.regex {
            let pattern = RegexBuilder::new(query)
                .case_insensitive(!options.match_case)
                .build()
                .ok();
            let invalid_regex = pattern.is_none();
            return Self {
                options,
                needle: None,
                pattern,
                invalid_regex,
            };
        }

        let needle = if options.match_case {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        Self {
            options,
            needle: Some(needle),
            pattern: None,
            invalid_regex: false,
        }
    }

    /// 搜索词为空（只输入了空白也算空）：此时不做任何过滤。
    pub(crate) fn is_empty(&self) -> bool {
        self.needle.is_none() && self.pattern.is_none() && !self.invalid_regex
    }

    /// 正则表达式写错了，搜索框应当给出提示。
    pub(crate) fn is_invalid_regex(&self) -> bool {
        self.invalid_regex
    }

    /// `text` 是否命中当前搜索。
    pub(crate) fn matches(&self, text: &str) -> bool {
        if let Some(pattern) = &self.pattern {
            return pattern.is_match(text);
        }
        let Some(needle) = &self.needle else {
            // 搜索词为空或不合法：不命中（空搜索词由调用方提前放行）。
            return false;
        };
        if !self.options.whole_word {
            return normalize(text, self.options.match_case).contains(needle);
        }
        split_words(text).any(|word| normalize(word, self.options.match_case) == *needle)
    }
}

/// 按匹配开关归一化文本：不区分大小写时统一转小写。
fn normalize(text: &str, match_case: bool) -> String {
    if match_case {
        text.to_string()
    } else {
        text.to_lowercase()
    }
}

/// 切分标识符里的词：分隔符（`_`、`.`、空格……）与驼峰都算边界。
fn split_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .flat_map(split_camel)
        .filter(|word| !word.is_empty())
}

/// 把 `bizMessage` 这类驼峰标识符拆成 `biz` 与 `Message`。
fn split_camel(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut start = 0;
    for index in 1..bytes.len() {
        let previous = bytes[index - 1];
        let current = bytes[index];
        let starts_new_word = (previous.is_ascii_lowercase() || previous.is_ascii_digit())
            && current.is_ascii_uppercase();
        if starts_new_word {
            words.push(&text[start..index]);
            start = index;
        }
    }
    if start < text.len() {
        words.push(&text[start..]);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(query: &str) -> TreeSearchMatcher {
        TreeSearchMatcher::new(query, TreeSearchOptions::default())
    }

    fn options(match_case: bool, whole_word: bool, regex: bool) -> TreeSearchOptions {
        TreeSearchOptions {
            match_case,
            whole_word,
            regex,
        }
    }

    #[test]
    fn an_empty_query_matches_nothing_but_reports_empty() {
        let matcher = plain("   ");

        assert!(matcher.is_empty());
        assert!(!matcher.matches("users"));
    }

    #[test]
    fn the_default_mode_ignores_case_and_matches_substrings() {
        let matcher = plain("USER");

        assert!(matcher.matches("biz_user_id"));
        assert!(matcher.matches("USERS"));
        assert!(!matcher.matches("order"));
    }

    #[test]
    fn match_case_only_accepts_the_exact_casing() {
        let matcher = TreeSearchMatcher::new("User", options(true, false, false));

        assert!(matcher.matches("UserTable"));
        assert!(!matcher.matches("user_table"));
    }

    #[test]
    fn whole_word_requires_the_entire_word_to_be_equal() {
        let matcher = TreeSearchMatcher::new("user", options(false, true, false));

        assert!(matcher.matches("biz_user_id"));
        assert!(!matcher.matches("users"));
        assert!(!matcher.matches("biz_userid"));
        // 驼峰也是词边界，所以 `User` 在这个名字里是一个完整的词。
        assert!(matcher.matches("bizUserTable"));
    }

    #[test]
    fn whole_word_treats_camel_case_and_separators_as_boundaries() {
        let matcher = TreeSearchMatcher::new("Message", options(false, true, false));

        assert!(matcher.matches("bizMessageCategory"));
        assert!(matcher.matches("biz.message"));
        assert!(!matcher.matches("bizMessages"));
    }

    #[test]
    fn whole_word_respects_the_case_switch() {
        let matcher = TreeSearchMatcher::new("Message", options(true, true, false));

        assert!(matcher.matches("bizMessageCategory"));
        assert!(!matcher.matches("bizmessage_category"));
    }

    #[test]
    fn regex_mode_matches_with_the_pattern() {
        let matcher = TreeSearchMatcher::new(r"^biz_(user|order)s?$", options(false, false, true));

        assert!(matcher.matches("biz_user"));
        assert!(matcher.matches("biz_orders"));
        assert!(!matcher.matches("app_biz_user"));
    }

    #[test]
    fn regex_mode_honours_the_case_switch() {
        let matcher = TreeSearchMatcher::new("User$", options(true, false, true));

        assert!(matcher.matches("biz_User"));
        assert!(!matcher.matches("biz_user"));
    }

    #[test]
    fn an_invalid_regex_matches_nothing_and_is_reported() {
        let matcher = TreeSearchMatcher::new("biz_(", options(false, false, true));

        assert!(matcher.is_invalid_regex());
        assert!(!matcher.is_empty());
        assert!(!matcher.matches("biz_("));
    }

    #[test]
    fn regex_mode_ignores_the_whole_word_switch() {
        let matcher = TreeSearchMatcher::new("user", options(false, true, true));

        // 正则优先：仍按正则的子串语义命中。
        assert!(matcher.matches("users"));
    }

    #[test]
    fn toggling_a_switch_only_flips_that_switch() {
        let options = TreeSearchOptions::default();

        let toggled = options.toggled(TreeSearchToggle::WholeWord);

        assert!(toggled.whole_word);
        assert!(!toggled.match_case);
        assert!(!toggled.regex);
        assert!(!toggled.toggled(TreeSearchToggle::WholeWord).whole_word);
    }

    #[test]
    fn the_whole_word_switch_is_disabled_while_the_regex_is_on() {
        let regex_on = TreeSearchOptions {
            regex: true,
            ..TreeSearchOptions::default()
        };

        assert!(regex_on.is_disabled(TreeSearchToggle::WholeWord));
        assert!(!regex_on.is_disabled(TreeSearchToggle::MatchCase));
        assert!(!regex_on.is_disabled(TreeSearchToggle::Regex));
        assert!(!TreeSearchOptions::default().is_disabled(TreeSearchToggle::WholeWord));
    }

    #[test]
    fn an_invalid_regex_is_only_reported_while_the_regex_switch_is_on() {
        let plain = TreeSearchOptions::default();
        let regex_on = TreeSearchOptions {
            regex: true,
            ..TreeSearchOptions::default()
        };

        assert!(regex_on.has_invalid_regex("users("));
        assert!(!regex_on.has_invalid_regex("users"));
        assert!(!plain.has_invalid_regex("users("));
        assert!(!TreeSearchOptions::default().has_invalid_regex("   "));
    }

    #[test]
    fn each_switch_has_its_own_label_and_element_id() {
        for toggle in TreeSearchToggle::ALL {
            let others: Vec<_> = TreeSearchToggle::ALL
                .into_iter()
                .filter(|other| *other != toggle)
                .collect();
            assert!(!others.iter().any(|other| other.label() == toggle.label()));
            assert!(
                !others
                    .iter()
                    .any(|other| other.element_id() == toggle.element_id())
            );
        }
        assert_eq!("Cc", TreeSearchToggle::MatchCase.label());
        assert_eq!("W", TreeSearchToggle::WholeWord.label());
        assert_eq!(".*", TreeSearchToggle::Regex.label());
    }

    #[test]
    fn the_switches_show_the_current_options_in_idea_order() {
        let options = TreeSearchOptions {
            match_case: true,
            whole_word: true,
            regex: true,
        };

        let views = search_toggle_views(options, false);

        assert_eq!(["Cc", "W", ".*"], views.each_ref().map(|view| view.label));
        assert_eq!(
            [true, true, true],
            views.each_ref().map(|view| view.checked)
        );
        // 正则模式下「全词」不可用，选项本身保留着。
        assert_eq!(
            [false, true, false],
            views.each_ref().map(|view| view.disabled)
        );
        assert_eq!(
            [false, false, false],
            views.each_ref().map(|view| view.invalid)
        );
    }

    #[test]
    fn clicking_a_switch_hands_over_the_toggled_options() {
        let options = TreeSearchOptions::default();

        let views = search_toggle_views(options, false);

        assert_eq!(options.toggled(TreeSearchToggle::MatchCase), views[0].next);
        assert_eq!(options.toggled(TreeSearchToggle::WholeWord), views[1].next);
        assert_eq!(options.toggled(TreeSearchToggle::Regex), views[2].next);
    }

    #[test]
    fn an_invalid_regex_is_marked_on_the_regex_switch_only() {
        let options = TreeSearchOptions {
            regex: true,
            ..TreeSearchOptions::default()
        };

        let views = search_toggle_views(options, true);

        assert_eq!(
            [false, false, true],
            views.each_ref().map(|view| view.invalid)
        );
        assert_eq!(
            t!("DbTreeView.search_regex_invalid").to_string(),
            views[2].tooltip
        );
        assert_eq!(
            t!("DbTreeView.search_regex").to_string(),
            search_toggle_views(options, false)[2].tooltip
        );
    }
}
