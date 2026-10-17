//! 终端型连接的启动参数解析:把 manifest 里的模板与连接表单保存的值合成为可直接启动的进程参数。
//!
//! 终端型连接没有提供者进程替它校验输入,而连接数据可能来自云同步 / 团队共享,
//! 所以这里承担两道宿主侧防线:
//! - `command` 固定,不允许占位符(见 [`ResourceConnectionTerminal::validate`]);
//! - 值不能把原本不是选项的参数变成选项(`-oProxyCommand=...` 之类的参数注入)。

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::connection::ResourceConnectionTerminal;

/// 解析后的终端启动参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTerminalLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// 标签标题;模板缺省或解析为空时为 `None`,由调用方回退到连接名称。
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TerminalLaunchError {
    #[error("invalid terminal template `{template}`: {reason}")]
    InvalidTemplate { template: String, reason: String },
    #[error("connection field `{field}` has an unsupported value")]
    UnsupportedValue { field: String },
    #[error("connection field `{field}` contains control characters")]
    ControlCharacters { field: String },
    #[error(
        "connection field `{field}` would turn argument `{template}` into a command-line option"
    )]
    OptionInjection { field: String, template: String },
}

/// 模板片段:字面量或字段引用。
enum Segment<'a> {
    Literal(&'a str),
    Field(&'a str),
}

fn parse_template(template: &str) -> Result<Vec<Segment<'_>>, TerminalLaunchError> {
    let invalid = |reason: &str| TerminalLaunchError::InvalidTemplate {
        template: template.to_string(),
        reason: reason.to_string(),
    };
    let mut segments = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            segments.push(Segment::Literal(&rest[..start]));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| invalid("unclosed `{{`"))?;
        let field = after[..end].trim();
        if field.is_empty() {
            return Err(invalid("empty placeholder"));
        }
        if field.contains("{{") {
            return Err(invalid("nested placeholder"));
        }
        segments.push(Segment::Field(field));
        rest = &after[end + 2..];
    }
    if rest.contains("}}") {
        return Err(invalid("unmatched `}}`"));
    }
    if !rest.is_empty() {
        segments.push(Segment::Literal(rest));
    }
    Ok(segments)
}

/// 模板引用的字段 ID(含语法检查),注册期用来校验引用的字段存在且非敏感。
pub(crate) fn template_fields(template: &str) -> Result<Vec<String>, TerminalLaunchError> {
    Ok(parse_template(template)?
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Field(field) => Some(field.to_string()),
            Segment::Literal(_) => None,
        })
        .collect())
}

/// 取字段值:缺省 / null 视为空;字符串、数字、布尔转文本;其余类型拒绝。
fn field_value(values: &Map<String, Value>, field: &str) -> Result<String, TerminalLaunchError> {
    let text = match values.get(field) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(_) => {
            return Err(TerminalLaunchError::UnsupportedValue {
                field: field.to_string(),
            });
        }
    };
    if text.chars().any(char::is_control) {
        return Err(TerminalLaunchError::ControlCharacters {
            field: field.to_string(),
        });
    }
    Ok(text)
}

/// 展开一个模板。返回 `(结果, 是否含占位符, 首个注入选项前缀的字段)`。
struct Expanded {
    text: String,
    has_placeholder: bool,
    /// 结果以 `-` 开头且来源于某个字段值时,记录该字段(用于选项注入判定)。
    leading_dash_field: Option<String>,
}

fn expand(template: &str, values: &Map<String, Value>) -> Result<Expanded, TerminalLaunchError> {
    let mut text = String::new();
    let mut has_placeholder = false;
    let mut leading_dash_field = None;
    for segment in parse_template(template)? {
        match segment {
            Segment::Literal(literal) => text.push_str(literal),
            Segment::Field(field) => {
                has_placeholder = true;
                let value = field_value(values, field)?;
                if text.is_empty() && value.starts_with('-') {
                    leading_dash_field = Some(field.to_string());
                }
                text.push_str(&value);
            }
        }
    }
    Ok(Expanded {
        text,
        has_placeholder,
        leading_dash_field,
    })
}

impl ResourceConnectionTerminal {
    /// 注册期校验模板本身(不涉及表单字段):命令非空且不含占位符,环境变量名合法,
    /// 模板语法正确。返回所有被引用的字段 ID,供调用方核对表单。
    pub(crate) fn validate(&self) -> Result<BTreeSet<String>, String> {
        let command = self.command.trim();
        if command.is_empty() {
            return Err("terminal command must not be empty".into());
        }
        if self.command.contains("{{") || self.command.contains("}}") {
            return Err("terminal command must not contain placeholders".into());
        }
        let mut referenced = BTreeSet::new();
        let templates = self
            .args
            .iter()
            .chain(self.env.values())
            .chain(self.title.iter());
        for template in templates {
            let fields = template_fields(template).map_err(|error| error.to_string())?;
            referenced.extend(fields);
        }
        for name in self.env.keys() {
            if name.is_empty() || name.contains(['=', '\0']) || name.contains("{{") {
                return Err(format!(
                    "invalid terminal environment variable name `{name}`"
                ));
            }
        }
        Ok(referenced)
    }

    /// 用连接表单保存的(非敏感)值解析出启动参数。
    ///
    /// - 含占位符的参数 / 环境变量解析为空时整项省略(可选字段不留空参数);
    /// - 模板不以 `-` 开头、而替换后以 `-` 开头的参数被拒绝(防选项注入)。
    pub fn resolve(
        &self,
        values: &Map<String, Value>,
    ) -> Result<ResolvedTerminalLaunch, TerminalLaunchError> {
        let mut args = Vec::with_capacity(self.args.len());
        for template in &self.args {
            let expanded = expand(template, values)?;
            if expanded.has_placeholder && expanded.text.is_empty() {
                continue;
            }
            if let Some(field) = expanded.leading_dash_field
                && !template.starts_with('-')
            {
                return Err(TerminalLaunchError::OptionInjection {
                    field,
                    template: template.clone(),
                });
            }
            args.push(expanded.text);
        }

        let mut env = Vec::with_capacity(self.env.len());
        for (name, template) in &self.env {
            let expanded = expand(template, values)?;
            if expanded.has_placeholder && expanded.text.is_empty() {
                continue;
            }
            env.push((name.clone(), expanded.text));
        }

        let title = match &self.title {
            Some(template) => Some(expand(template, values)?.text).filter(|text| !text.is_empty()),
            None => None,
        };

        Ok(ResolvedTerminalLaunch {
            command: self.command.trim().to_string(),
            args,
            env,
            title,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn terminal(
        args: &[&str],
        env: &[(&str, &str)],
        title: Option<&str>,
    ) -> ResourceConnectionTerminal {
        ResourceConnectionTerminal {
            command: "tsh".into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>(),
            title: title.map(str::to_string),
        }
    }

    fn values(value: serde_json::Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn resolve_substitutes_args_env_and_title() {
        let launch = terminal(
            &["ssh", "{{target}}"],
            &[("TELEPORT_PROXY", "{{proxy}}")],
            Some("{{target}}"),
        )
        .resolve(&values(
            json!({"target": " root@db-1 ", "proxy": "tp.example.com:443"}),
        ))
        .unwrap();
        assert_eq!("tsh", launch.command);
        assert_eq!(vec!["ssh", "root@db-1"], launch.args);
        assert_eq!(
            vec![(
                "TELEPORT_PROXY".to_string(),
                "tp.example.com:443".to_string()
            )],
            launch.env
        );
        assert_eq!(Some("root@db-1".to_string()), launch.title);
    }

    #[test]
    fn resolve_omits_empty_optional_env_and_args() {
        let launch = terminal(
            &["ssh", "{{extra}}", "{{target}}"],
            &[("TELEPORT_CLUSTER", "{{cluster}}"), ("FIXED", "1")],
            None,
        )
        .resolve(&values(json!({"target": "db-1", "cluster": "  "})))
        .unwrap();
        assert_eq!(vec!["ssh", "db-1"], launch.args);
        assert_eq!(vec![("FIXED".to_string(), "1".to_string())], launch.env);
        assert_eq!(None, launch.title);
    }

    #[test]
    fn resolve_rejects_value_that_becomes_an_option() {
        let error = terminal(&["ssh", "{{target}}"], &[], None)
            .resolve(&values(json!({"target": "-oProxyCommand=touch /tmp/x"})))
            .unwrap_err();
        assert_eq!(
            TerminalLaunchError::OptionInjection {
                field: "target".into(),
                template: "{{target}}".into(),
            },
            error
        );
    }

    #[test]
    fn resolve_rejects_option_injection_through_leading_placeholder() {
        let error = terminal(&["{{user}}@{{host}}"], &[], None)
            .resolve(&values(json!({"user": "-oFoo", "host": "h"})))
            .unwrap_err();
        assert!(matches!(error, TerminalLaunchError::OptionInjection { .. }));
    }

    #[test]
    fn resolve_allows_dash_when_template_is_already_an_option() {
        let launch = terminal(&["--host={{target}}", "-l"], &[], None)
            .resolve(&values(json!({"target": "-weird"})))
            .unwrap();
        assert_eq!(vec!["--host=-weird", "-l"], launch.args);
    }

    #[test]
    fn resolve_allows_dash_inside_value() {
        let launch = terminal(&["ssh", "{{target}}"], &[], None)
            .resolve(&values(json!({"target": "redis-sentinel-dev1"})))
            .unwrap();
        assert_eq!(vec!["ssh", "redis-sentinel-dev1"], launch.args);
    }

    #[test]
    fn resolve_rejects_control_characters_and_composite_values() {
        let template = terminal(&["{{target}}"], &[], None);
        assert_eq!(
            Err(TerminalLaunchError::ControlCharacters {
                field: "target".into()
            }),
            template.resolve(&values(json!({"target": "a\nb"})))
        );
        assert_eq!(
            Err(TerminalLaunchError::UnsupportedValue {
                field: "target".into()
            }),
            template.resolve(&values(json!({"target": {"x": 1}})))
        );
    }

    #[test]
    fn validate_collects_referenced_fields() {
        let referenced = terminal(
            &["ssh", "{{ target }}"],
            &[("A", "{{proxy}}")],
            Some("{{target}}"),
        )
        .validate()
        .unwrap();
        assert_eq!(
            BTreeSet::from(["proxy".to_string(), "target".to_string()]),
            referenced
        );
    }

    #[test]
    fn validate_rejects_bad_command_templates_and_env_names() {
        let mut bad = terminal(&["ssh"], &[], None);
        bad.command = "  ".into();
        assert!(bad.validate().is_err());
        bad.command = "{{tool}}".into();
        assert!(bad.validate().is_err());
        assert!(terminal(&["{{open"], &[], None).validate().is_err());
        assert!(terminal(&["{{}}"], &[], None).validate().is_err());
        assert!(terminal(&["a}}"], &[], None).validate().is_err());
        assert!(terminal(&[], &[("A=B", "x")], None).validate().is_err());
    }
}
