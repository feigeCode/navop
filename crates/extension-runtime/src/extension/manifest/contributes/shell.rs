use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellSurface {
    /// 从扩展管理页或 shell 入口打开的普通 tab 视图。
    #[default]
    Tab,
    /// 工具箱页面聚合的小工具卡片；与连接扩展(contributes.connections)
    /// 区分：无连接表单、无 shellViewId 关联、面向单机小工具。
    Toolbox,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShellHostModule {
    Context,
    Resource,
    Job,
    Event,
    Blob,
    Log,
    Runtime,
    /// 开发者工具专用（navop.dev host 模块）。
    Dev,
    /// 宿主界面语言（`navop.i18n` host 模块）。
    ///
    /// 只给「宿主当前是什么语言」这一个事实与它的变更信号；**不含词条表** ——
    /// 扩展文案不在宿主的 rust-i18n 目录里，词条由扩展自己带。
    #[serde(rename = "i18n")]
    I18n,
    /// 宿主的组件主题（`navop.theme` host 模块）。
    ///
    /// `cx.theme()` 给的是 gpui-base 的 18 个语义色；宿主界面真正在用的
    /// `list_active` / `table_hover` / `sidebar_*` 这些面只在这个模块里。
    #[serde(rename = "theme")]
    Theme,
}

impl ShellHostModule {
    pub fn requires_backend(self) -> bool {
        matches!(self, Self::Resource | Self::Job | Self::Event | Self::Blob)
    }

    /// 该模块在脚本里的 import specifier：`"navop.i18n"` 对应
    /// `import { locale } from "navop.i18n"`。
    ///
    /// 刻意写成**无通配臂**的 `match` —— 给枚举加一个变体时这里会先编译失败，
    /// 逼作者表态"这个模块的脚本名字是什么"。少了这道闸，漏掉的名字不会在
    /// 编译期或安装期暴露：扩展装得上（`validate_modules` 认 manifest 那个
    /// 拼写），失败点被推到页面运行期的 `import` 解析上，表现是整页白屏。
    pub const fn module_specifier(self) -> &'static str {
        match self {
            Self::Context => "navop.context",
            Self::Resource => "navop.resource",
            Self::Job => "navop.job",
            Self::Event => "navop.event",
            Self::Blob => "navop.blob",
            Self::Log => "navop.log",
            Self::Runtime => "navop.runtime",
            Self::Dev => "navop.dev",
            Self::I18n => "navop.i18n",
            Self::Theme => "navop.theme",
        }
    }

    /// 全部变体，按声明顺序。守卫要遍历声明面时用这里，别在测试里另抄一份。
    ///
    /// ⚠️ 这份清单**没有编译期完备性保证**（`std::mem::variant_count` 到
    /// rustc 1.98 仍是 unstable 特性），所以它紧挨着上面那个 `match` 放：
    /// 新增变体时那个 `match` 必然编译失败，动它的人一定看得见这里。
    pub const ALL: &'static [Self] = &[
        Self::Context,
        Self::Resource,
        Self::Job,
        Self::Event,
        Self::Blob,
        Self::Log,
        Self::Runtime,
        Self::Dev,
        Self::I18n,
        Self::Theme,
    ];
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellViewContrib {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub surface: ShellSurface,
    #[serde(default)]
    pub singleton: bool,
    #[serde(default)]
    pub backends: BTreeMap<String, String>,
    #[serde(default)]
    pub modules: Vec<ShellHostModule>,
    /// toolbox surface 专用：卡片所属分类（如 `text`、`network`、`system`）。
    /// `tab` surface 忽略该字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// toolbox surface 专用：搜索关键词，补充 title/description 匹配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::ShellHostModule;

    /// 声明面拼写 ↔ 脚本 import specifier 是**同一条链子的两半**，都由本文件
    /// 决定却互不引用：左边来自 `#[serde(rename)]`，右边来自
    /// `module_specifier`。一边改名、另一边没跟上，扩展照样装得上
    /// （`validate_modules` 认 manifest 那个拼写），页面里的 `import` 却在运行期
    /// 解析不到 ⇒ 整页白屏。所以这条链子得钉死，不能靠"看着差不多"。
    #[test]
    fn every_host_module_specifier_matches_its_manifest_spelling() {
        let mut seen = HashSet::new();
        for module in ShellHostModule::ALL {
            let specifier = module.module_specifier();
            let script_name = specifier
                .strip_prefix("navop.")
                .unwrap_or_else(|| panic!("`{specifier}` 不在 navop 命名空间里"));
            assert!(
                seen.insert(specifier),
                "两个变体共用了同一个 specifier `{specifier}`"
            );
            let declared = serde_json::to_value(module).expect("host module serializes");
            assert_eq!(
                declared,
                serde_json::json!(script_name),
                "manifest 拼写与脚本 import 的名字漂了：{specifier}"
            );
        }
    }

    /// 两个新模块的定位：**脚本要给名字，但不许要后端**。
    ///
    /// 「不许要后端」不是风格问题：`validate_modules` 对「声明了
    /// `requires_backend` 的模块 + `backends` 为空」判安装期失败，而这两个模块
    /// 的目标场景恰恰是纯界面页（只有文案和配色，没有任何连接）—— 误算进那一
    /// 类，这类页面连装都装不上。
    #[test]
    fn language_and_theme_are_declared_but_never_require_a_backend() {
        assert_eq!("navop.i18n", ShellHostModule::I18n.module_specifier());
        assert_eq!("navop.theme", ShellHostModule::Theme.module_specifier());
        assert!(!ShellHostModule::I18n.requires_backend());
        assert!(!ShellHostModule::Theme.requires_backend());
    }
}
