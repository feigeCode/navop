use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShellHostModule {
    /// ⚠️ ABI v2 已删除该模块：连接参数改从 `host.params` 读。
    ///
    /// 变体保留只为让老清单落到 `validate_modules` 的可读安装期错误（serde
    /// 认这个拼写）；它不在 [`ShellHostModule::is_supported`] 里，因此永远
    /// 派生不出 host 能力。
    Context,
    Resource,
    Job,
    Event,
    Blob,
    Log,
    Runtime,
    /// 开发者工具专用（navop.dev host 模块）。
    ///
    /// 只对内置扩展开放：`validate_modules` 在安装期按 id 白名单拒绝第三方声明。
    Dev,
    /// ⚠️ ABI v2 已删除该模块：界面语言由 nsh 原生提供。
    ///
    /// 变体保留只为让老清单报安装期错误，见 [`ShellHostModule::is_supported`]。
    #[serde(rename = "i18n")]
    I18n,
    /// ⚠️ ABI v2 已删除该模块：组件主题由 nsh 原生提供。
    ///
    /// 变体保留只为让老清单报安装期错误，见 [`ShellHostModule::is_supported`]。
    #[serde(rename = "theme")]
    Theme,
}

impl ShellHostModule {
    pub fn requires_backend(self) -> bool {
        matches!(self, Self::Resource | Self::Job | Self::Event | Self::Blob)
    }

    /// 宿主当前支持的模块集合（ABI v2，spec §7）。
    ///
    /// 唯一的支持面判定入口：`validate_modules` 用它做安装期拒绝，
    /// `nsh_host::grant` 用它决定派生哪些 host 能力。`Context` / `I18n` /
    /// `Theme` 是 v2 删掉的旧值——保留枚举变体只是为了报错清楚，三者一律
    /// 返回 `false`，不会出现在任何一处分发里。
    pub const fn is_supported(self) -> bool {
        matches!(
            self,
            Self::Resource
                | Self::Job
                | Self::Event
                | Self::Blob
                | Self::Log
                | Self::Runtime
                | Self::Dev
        )
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
pub struct ShellAppContrib {
    /// tab 标题；缺省回退清单的 `name`。
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// nsh 程序的入口，相对于扩展根（通常是 `ui/dist/bundle.js`）。
    pub entry: String,
    /// 别名 → 本清单里 `runtime.ipc[].id`。
    #[serde(default)]
    pub backends: BTreeMap<String, String>,
    #[serde(default)]
    pub modules: Vec<ShellHostModule>,
    /// 工具箱卡片分组（没有连接的 app 才进工具箱）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
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

    /// v2 的支持面就是这七个模块，且顺序稳定（`ALL` 的声明顺序）。
    ///
    /// `context` / `i18n` / `theme` 是 v2 删除的旧值：枚举变体保留只为让老清单
    /// 在 `validate_modules` 里落到一句可读的「已删除」错误，它们不能算支持面，
    /// 也不能再派生 host 能力。
    #[test]
    fn only_the_v2_modules_are_supported() {
        let supported = ShellHostModule::ALL
            .iter()
            .filter(|module| module.is_supported())
            .map(|module| module.module_specifier())
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                "navop.resource",
                "navop.job",
                "navop.event",
                "navop.blob",
                "navop.log",
                "navop.runtime",
                "navop.dev",
            ],
            supported
        );
        for legacy in [
            ShellHostModule::Context,
            ShellHostModule::I18n,
            ShellHostModule::Theme,
        ] {
            assert!(
                !legacy.is_supported(),
                "{} 不应算 v2 支持面",
                legacy.module_specifier()
            );
        }
    }

    /// i18n/theme 已从 v2 删除，但枚举契约保持不变：旧清单反序列化出来的值
    /// 仍然不需要后端（报错路径不依赖这条，保留它是为了防止枚举语义漂移）。
    #[test]
    fn language_and_theme_are_declared_but_never_require_a_backend() {
        assert_eq!("navop.i18n", ShellHostModule::I18n.module_specifier());
        assert_eq!("navop.theme", ShellHostModule::Theme.module_specifier());
        assert!(!ShellHostModule::I18n.requires_backend());
        assert!(!ShellHostModule::Theme.requires_backend());
    }
}
