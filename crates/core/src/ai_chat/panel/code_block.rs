use gpui::{App, Hsla, SharedString, Window};
use gpui_component::IconName;
use std::sync::Arc;

/// AI 聊天面板的自定义颜色配置
///
/// 用于在终端等需要自定义主题的场景中覆盖默认颜色
#[derive(Clone, Debug)]
pub struct AiChatColors {
    /// 主背景色
    pub background: Hsla,
    /// 主前景色（文字）
    pub foreground: Hsla,
    /// 次要背景色（卡片、列表项）
    pub muted: Hsla,
    /// 次要前景色（占位符、次要文字）
    pub muted_foreground: Hsla,
    /// 边框色
    pub border: Hsla,
    /// 强调背景色
    pub accent: Hsla,
    /// 强调前景色
    pub accent_foreground: Hsla,
}

// ============================================================================
// 代码块操作扩展机制
// ============================================================================

/// 语言匹配器 - 用于匹配代码块的语言类型
#[derive(Clone)]
pub enum LanguageMatcher {
    /// 精确匹配（不区分大小写）
    Exact(Vec<&'static str>),
    /// 前缀匹配
    Prefix(&'static str),
    /// 自定义匹配函数
    Custom(Arc<dyn Fn(&str) -> bool + Send + Sync>),
    /// 匹配所有语言（包括未指定语言的代码块）
    Any,
}

impl LanguageMatcher {
    /// 创建精确匹配器（单个语言）
    pub fn exact(lang: &'static str) -> Self {
        Self::Exact(vec![lang])
    }

    /// 创建精确匹配器（多个语言）
    pub fn exact_many(langs: Vec<&'static str>) -> Self {
        Self::Exact(langs)
    }

    /// 创建 SQL 语言匹配器
    pub fn sql() -> Self {
        Self::Exact(vec![
            "sql",
            "mysql",
            "postgresql",
            "postgres",
            "sqlite",
            "mssql",
            "oracle",
            "plsql",
        ])
    }

    /// 创建 Shell/Bash 语言匹配器
    pub fn shell() -> Self {
        Self::Exact(vec![
            "bash",
            "sh",
            "shell",
            "zsh",
            "fish",
            "powershell",
            "ps1",
            "cmd",
            "batch",
        ])
    }

    /// 创建 Python 语言匹配器
    pub fn python() -> Self {
        Self::Exact(vec!["python", "py", "python3"])
    }

    /// 创建 Rust 语言匹配器
    pub fn rust() -> Self {
        Self::Exact(vec!["rust", "rs"])
    }

    /// 创建 JavaScript/TypeScript 语言匹配器
    pub fn javascript() -> Self {
        Self::Exact(vec!["javascript", "js", "typescript", "ts", "jsx", "tsx"])
    }

    /// 检查是否匹配给定的语言
    pub fn matches(&self, lang: Option<&str>) -> bool {
        match self {
            LanguageMatcher::Exact(langs) => lang.map_or(false, |l| {
                let l_lower = l.to_lowercase();
                langs
                    .iter()
                    .any(|&expected| expected.eq_ignore_ascii_case(&l_lower))
            }),
            LanguageMatcher::Prefix(prefix) => lang.map_or(false, |l| {
                l.to_lowercase().starts_with(&prefix.to_lowercase())
            }),
            LanguageMatcher::Custom(f) => lang.map_or(false, |l| f(l)),
            LanguageMatcher::Any => true,
        }
    }
}

/// 代码块操作回调函数类型
///
/// 参数：
/// - `code`: 代码块内容
/// - `lang`: 代码块语言（可能为空）
/// - `window`: 窗口引用
/// - `cx`: 应用上下文
pub type CodeBlockActionCallback =
    Arc<dyn Fn(String, Option<String>, &mut Window, &mut App) + Send + Sync>;

/// 代码块操作定义
///
/// 用于定义一个可以在代码块上执行的操作，例如：
/// - SQL 代码发送到编辑器
/// - Shell 命令复制到终端
/// - Python 代码直接运行
#[derive(Clone)]
pub struct CodeBlockAction {
    /// 唯一标识符
    pub id: SharedString,
    /// 显示图标
    pub icon: IconName,
    /// 按钮标签（可选，如果为 None 则只显示图标）
    pub label: Option<SharedString>,
    /// 语言匹配器
    pub matcher: LanguageMatcher,
    /// 操作回调
    pub callback: CodeBlockActionCallback,
}

impl CodeBlockAction {
    /// 创建新的代码块操作
    pub fn new(id: impl Into<SharedString>) -> CodeBlockActionBuilder {
        CodeBlockActionBuilder {
            id: id.into(),
            icon: IconName::SquareTerminal,
            label: None,
            matcher: LanguageMatcher::Any,
            callback: None,
        }
    }
}

/// 代码块操作构建器
pub struct CodeBlockActionBuilder {
    id: SharedString,
    icon: IconName,
    label: Option<SharedString>,
    matcher: LanguageMatcher,
    callback: Option<CodeBlockActionCallback>,
}

impl CodeBlockActionBuilder {
    /// 设置图标
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    /// 设置标签
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// 设置语言匹配器
    pub fn matcher(mut self, matcher: LanguageMatcher) -> Self {
        self.matcher = matcher;
        self
    }

    /// 设置回调函数
    pub fn on_click<F>(mut self, f: F) -> Self
    where
        F: Fn(String, Option<String>, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.callback = Some(Arc::new(f));
        self
    }

    /// 构建代码块操作
    pub fn build(self) -> Option<CodeBlockAction> {
        self.callback.map(|callback| CodeBlockAction {
            id: self.id,
            icon: self.icon,
            label: self.label,
            matcher: self.matcher,
            callback,
        })
    }
}

/// 代码块操作注册表
///
/// 用于管理和查询已注册的代码块操作
#[derive(Clone, Default)]
pub struct CodeBlockActionRegistry {
    actions: Vec<CodeBlockAction>,
}

impl CodeBlockActionRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// 注册一个代码块操作
    pub fn register(&mut self, action: CodeBlockAction) {
        self.actions.push(action);
    }

    /// 获取匹配指定语言的所有操作
    pub fn get_actions_for_lang(&self, lang: Option<&str>) -> Vec<&CodeBlockAction> {
        self.actions
            .iter()
            .filter(|action| action.matcher.matches(lang))
            .collect()
    }

    /// 检查是否有注册的操作
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// 获取所有操作数量
    pub fn len(&self) -> usize {
        self.actions.len()
    }
}
