//! 接口测试脚本执行引擎（移植自 verve 的 `src/scripting.rs`）。
//!
//! 脚本是运行在沙箱 [boa_engine] 中的 JavaScript，`apt` 全局对象提供：
//!
//! - `apt.variables.set(name, value)` / `apt.variables.get(name)`
//! - `apt.setVariable(name, value)` / `apt.getVariable(name)`（别名）
//! - `apt.environment.set(name, value)` / `apt.environment.get(name)`
//! - `apt.assert(condition, message?)` —— 记录一条断言结果
//! - `apt.echo(...)` / `console.log(...)` —— 捕获控制台输出
//!
//! 后置测试脚本中还有全局 `response` 对象：
//!
//! ```js
//! response.status   // number
//! response.body     // string（JSON 响应会美化，其它响应保持原文）
//! response.json     // parsed object（响应体为 JSON 时）
//! response.headers  // object {key: value}
//! response.time     // number (ms)
//! response.ok       // boolean（2xx 且没有协议/传输错误）
//! response.error    // string | undefined
//! ```

use std::collections::BTreeMap;

use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{Context, JsArgs, JsValue, NativeFunction, Source, js_string};

use crate::http::HttpResponse;

const SCRIPT_LOOP_ITERATION_LIMIT: u64 = 100_000;

/// 变量写入的目标作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarScope {
    /// 全局/环境作用域（跨请求保留）。
    Environment,
    /// 单次请求作用域。
    Request,
}

/// 脚本执行产生的副作用。
#[derive(Debug, Clone)]
pub enum SideEffect {
    SetVariable {
        scope: VarScope,
        name: String,
        value: String,
    },
    /// 一条控制台输出（`apt.echo` / `console.log`）。
    Log(String),
    /// 一条断言结果。
    Assert { passed: bool, message: String },
}

/// 一次脚本运行收集到的结果。
#[derive(Debug, Default, Clone)]
pub struct ScriptResult {
    pub effects: Vec<SideEffect>,
    /// 控制台 + 断言输出行（用于展示）。
    pub logs: Vec<String>,
    pub assertions_passed: usize,
    pub assertions_failed: usize,
    /// JS 运行时错误（脚本抛出时）。
    pub error: Option<String>,
}

/// 运行前置脚本（无 `response` 对象）。
pub fn run_pre_request(script: &str, vars: &BTreeMap<String, String>) -> ScriptResult {
    run(script, vars, None)
}

/// 运行后置测试脚本，`response` 全局对象由捕获的 [`HttpResponse`] 填充。
pub fn run_post_request(
    script: &str,
    vars: &BTreeMap<String, String>,
    response: &HttpResponse,
) -> ScriptResult {
    run(script, vars, Some(response))
}

/// 运行独立脚本（无 `response` 对象）。
pub fn run_standalone_script(script: &str, vars: &BTreeMap<String, String>) -> ScriptResult {
    run(script, vars, None)
}

fn run(
    script: &str,
    vars: &BTreeMap<String, String>,
    response: Option<&HttpResponse>,
) -> ScriptResult {
    if script.trim().is_empty() {
        return ScriptResult::default();
    }
    let mut ctx = Context::default();
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(SCRIPT_LOOP_ITERATION_LIMIT);

    // 通过全局属性把变量池和副作用列表传给 fn-ptr 原生函数（无法捕获状态）。
    let vars_json = serde_json::to_string(vars).unwrap_or_else(|_| "{}".into());
    let _ = ctx.register_global_property(
        js_string!("__verve_vars__"),
        JsValue::from(js_string!(vars_json.as_str())),
        Attribute::all(),
    );
    let _ = ctx.register_global_property(
        js_string!("__verve_effects__"),
        JsValue::from(js_string!("")),
        Attribute::all(),
    );

    if let Err(e) = install_apt(&mut ctx) {
        return ScriptResult {
            error: Some(format!("failed to init script runtime: {e}")),
            ..Default::default()
        };
    }
    if let Err(e) = install_response(&mut ctx, response) {
        return ScriptResult {
            error: Some(format!("failed to init response object: {e}")),
            ..Default::default()
        };
    }

    let eval_result = ctx.eval(Source::from_bytes(script));
    let collected = collect_effects(&mut ctx);

    let mut result = ScriptResult::default();
    for effect in &collected {
        match effect {
            SideEffect::Log(line) => result.logs.push(line.clone()),
            SideEffect::Assert { passed, message } => {
                if *passed {
                    result.assertions_passed += 1;
                } else {
                    result.assertions_failed += 1;
                }
                result.logs.push(format!(
                    "{}: {message}",
                    if *passed { "✓ PASS" } else { "✗ FAIL" }
                ));
            }
            SideEffect::SetVariable { .. } => {}
        }
    }
    result.effects = collected;
    if let Err(e) = eval_result {
        result.error = Some(e.to_string());
    }
    result
}

/// 安装 `apt` 全局对象和 `console`。
fn install_apt(ctx: &mut Context) -> Result<(), boa_engine::JsError> {
    let console = {
        let mut obj = ObjectInitializer::new(ctx);
        obj.function(
            NativeFunction::from_fn_ptr(log_native),
            js_string!("log"),
            0,
        );
        obj.build()
    };
    ctx.register_global_property(js_string!("console"), console, Attribute::all())?;

    let variables = {
        let mut obj = ObjectInitializer::new(ctx);
        obj.function(
            NativeFunction::from_fn_ptr(var_get_native),
            js_string!("get"),
            1,
        );
        obj.function(
            NativeFunction::from_fn_ptr(var_set_request_native),
            js_string!("set"),
            2,
        );
        obj.build()
    };
    let environment = {
        let mut obj = ObjectInitializer::new(ctx);
        obj.function(
            NativeFunction::from_fn_ptr(var_get_native),
            js_string!("get"),
            1,
        );
        obj.function(
            NativeFunction::from_fn_ptr(var_set_env_native),
            js_string!("set"),
            2,
        );
        obj.build()
    };

    let mut apt = ObjectInitializer::new(ctx);
    apt.function(
        NativeFunction::from_fn_ptr(echo_native),
        js_string!("echo"),
        0,
    );
    apt.function(
        NativeFunction::from_fn_ptr(assert_native),
        js_string!("assert"),
        2,
    );
    apt.property(js_string!("variables"), variables, Attribute::all());
    apt.property(js_string!("environment"), environment, Attribute::all());
    apt.function(
        NativeFunction::from_fn_ptr(var_set_request_native),
        js_string!("setVariable"),
        2,
    );
    apt.function(
        NativeFunction::from_fn_ptr(var_get_native),
        js_string!("getVariable"),
        1,
    );
    let apt_obj = apt.build();

    ctx.register_global_property(js_string!("apt"), apt_obj, Attribute::all())?;
    Ok(())
}

/// 为后置脚本安装全局 `response` 对象。
fn install_response(
    ctx: &mut Context,
    response: Option<&HttpResponse>,
) -> Result<(), boa_engine::JsError> {
    let json_val: Option<JsValue> = match response {
        Some(r) if r.is_json => serde_json::from_str::<serde_json::Value>(&r.body)
            .ok()
            .map(|v| json_to_js_value(ctx, &v))
            .transpose()?,
        _ => None,
    };

    let resp_obj = ObjectInitializer::new(ctx).build();
    if let Some(r) = response {
        let ok = r.error.is_none() && (200..300).contains(&r.status);
        let _ = resp_obj.set(
            js_string!("status"),
            JsValue::from(r.status as f64),
            false,
            ctx,
        );
        let _ = resp_obj.set(
            js_string!("time"),
            JsValue::from(r.time_ms as f64),
            false,
            ctx,
        );
        let _ = resp_obj.set(
            js_string!("body"),
            JsValue::from(js_string!(r.body.as_str())),
            false,
            ctx,
        );
        let _ = resp_obj.set(js_string!("ok"), JsValue::from(ok), false, ctx);
        let error = r.error.as_ref().map_or_else(JsValue::undefined, |error| {
            JsValue::from(js_string!(error.as_str()))
        });
        let _ = resp_obj.set(js_string!("error"), error, false, ctx);
        if let Some(jv) = json_val {
            let _ = resp_obj.set(js_string!("json"), jv, false, ctx);
        }
        let headers = ObjectInitializer::new(ctx).build();
        for kv in &r.headers {
            let _ = headers.set(
                js_string!(kv.key.as_str()),
                JsValue::from(js_string!(kv.value.as_str())),
                false,
                ctx,
            );
        }
        let _ = resp_obj.set(js_string!("headers"), headers, false, ctx);
    } else {
        let _ = resp_obj.set(js_string!("status"), JsValue::from(0.0f64), false, ctx);
        let _ = resp_obj.set(js_string!("body"), JsValue::undefined(), false, ctx);
        let _ = resp_obj.set(js_string!("ok"), JsValue::from(false), false, ctx);
        let _ = resp_obj.set(js_string!("error"), JsValue::undefined(), false, ctx);
    }
    ctx.register_global_property(js_string!("response"), resp_obj, Attribute::all())?;
    Ok(())
}

/// 把 serde_json::Value 转成 boa JsValue。
fn json_to_js_value(
    ctx: &mut Context,
    value: &serde_json::Value,
) -> Result<JsValue, boa_engine::JsError> {
    Ok(match value {
        serde_json::Value::Null => JsValue::null(),
        serde_json::Value::Bool(b) => JsValue::from(*b),
        serde_json::Value::Number(n) => JsValue::from(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => JsValue::from(js_string!(s.as_str())),
        serde_json::Value::Array(arr) => {
            let array = boa_engine::object::builtins::JsArray::new(ctx);
            for v in arr {
                array.push(json_to_js_value(ctx, v)?, ctx)?;
            }
            array.into()
        }
        serde_json::Value::Object(map) => {
            let obj = ObjectInitializer::new(ctx).build();
            for (k, v) in map {
                let val = json_to_js_value(ctx, v)?;
                let _ = obj.set(js_string!(k.as_str()), val, false, ctx);
            }
            obj.into()
        }
    })
}

// ---------------------------------------------------------------------------
// 原生函数（fn 指针，无捕获；状态通过 context 全局读写）。
// ---------------------------------------------------------------------------

fn log_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let msg = join_args(args, ctx);
    push_effect(ctx, SideEffect::Log(msg));
    Ok(JsValue::undefined())
}

fn echo_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let msg = join_args(args, ctx);
    push_effect(ctx, SideEffect::Log(msg));
    Ok(JsValue::undefined())
}

fn assert_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let passed = args.get_or_undefined(0).to_boolean();
    let message = args
        .get(1)
        .and_then(|v| {
            if v.is_undefined() {
                None
            } else {
                v.to_string(ctx).ok().map(|s| s.to_std_string_escaped())
            }
        })
        .unwrap_or_else(|| "assertion".to_string());
    push_effect(ctx, SideEffect::Assert { passed, message });
    Ok(JsValue::undefined())
}

fn var_get_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let name = arg_string(args, 0, ctx);
    let value = current_var(ctx, &name);
    Ok(JsValue::from(js_string!(value.as_str())))
}

fn var_set_env_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let name = arg_string(args, 0, ctx);
    let value = arg_string(args, 1, ctx);
    set_current_var(ctx, &name, &value);
    push_effect(
        ctx,
        SideEffect::SetVariable {
            scope: VarScope::Environment,
            name,
            value,
        },
    );
    Ok(JsValue::undefined())
}

fn var_set_request_native(
    _this: &JsValue,
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<JsValue, boa_engine::JsError> {
    let name = arg_string(args, 0, ctx);
    let value = arg_string(args, 1, ctx);
    set_current_var(ctx, &name, &value);
    push_effect(
        ctx,
        SideEffect::SetVariable {
            scope: VarScope::Request,
            name,
            value,
        },
    );
    Ok(JsValue::undefined())
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

fn join_args(args: &[JsValue], ctx: &mut Context) -> String {
    args.iter()
        .map(|v| {
            if v.is_undefined() {
                "undefined".to_string()
            } else {
                v.to_string(ctx)
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn arg_string(args: &[JsValue], ix: usize, ctx: &mut Context) -> String {
    args.get_or_undefined(ix)
        .to_string(ctx)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default()
}

/// 序列化一条副作用并追加到 `__verve_effects__` 全局字符串。
fn push_effect(ctx: &mut Context, effect: SideEffect) {
    let tag = match &effect {
        SideEffect::Log(msg) => format!("LOG\x1f{}", escape(msg)),
        SideEffect::Assert { passed, message } => {
            format!("ASSERT\x1f{passed}\x1f{}", escape(message))
        }
        SideEffect::SetVariable { scope, name, value } => {
            let s = if scope == &VarScope::Environment {
                "env"
            } else {
                "req"
            };
            format!("SET\x1f{s}\x1f{}\x1f{}", escape(name), escape(value))
        }
    };
    let cur = read_global_string(ctx, "__verve_effects__");
    let next = if cur.is_empty() {
        tag
    } else {
        format!("{cur}\x1e{tag}")
    };
    write_global_string(ctx, "__verve_effects__", &next);
}

/// 运行结束后读回副作用列表。
fn collect_effects(ctx: &mut Context) -> Vec<SideEffect> {
    let raw = read_global_string(ctx, "__verve_effects__");
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('\x1e')
        .filter_map(|chunk| {
            let mut parts = chunk.splitn(4, '\x1f');
            match parts.next()? {
                "LOG" => Some(SideEffect::Log(unescape(parts.next().unwrap_or("")))),
                "ASSERT" => {
                    let passed = parts.next() == Some("true");
                    let message = unescape(parts.next().unwrap_or("assertion"));
                    Some(SideEffect::Assert { passed, message })
                }
                "SET" => {
                    let scope_s = parts.next().unwrap_or("env");
                    let name = unescape(parts.next().unwrap_or(""));
                    let value = unescape(parts.next().unwrap_or(""));
                    let scope = if scope_s == "req" {
                        VarScope::Request
                    } else {
                        VarScope::Environment
                    };
                    Some(SideEffect::SetVariable { scope, name, value })
                }
                _ => None,
            }
        })
        .collect()
}

fn current_var(ctx: &mut Context, name: &str) -> String {
    let json = read_global_string(ctx, "__verve_vars__");
    let map: BTreeMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
    map.get(name).cloned().unwrap_or_default()
}

fn set_current_var(ctx: &mut Context, name: &str, value: &str) {
    let json = read_global_string(ctx, "__verve_vars__");
    let mut map: BTreeMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
    map.insert(name.to_string(), value.to_string());
    let next = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
    write_global_string(ctx, "__verve_vars__", &next);
}

fn read_global_string(ctx: &mut Context, key: &str) -> String {
    ctx.global_object()
        .get(js_string!(key), ctx)
        .ok()
        .and_then(|v| {
            v.as_string()
                .map(|s| s.to_std_string_escaped())
                .or_else(|| v.as_number().map(|n| n.to_string()))
        })
        .unwrap_or_default()
}

fn write_global_string(ctx: &mut Context, key: &str, value: &str) {
    let _ = ctx.global_object().set(
        boa_engine::property::PropertyKey::String(js_string!(key)),
        JsValue::from(js_string!(value)),
        false,
        ctx,
    );
}

fn escape(s: &str) -> String {
    s.replace('\x1e', "\u{FFFE}").replace('\x1f', "\u{FFFD}")
}

fn unescape(s: &str) -> String {
    s.replace('\u{FFFE}', "\x1e").replace('\u{FFFD}', "\x1f")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn response() -> HttpResponse {
        HttpResponse {
            status: 200,
            status_text: "OK".into(),
            time_ms: 12,
            body: r#"{"code":0,"token":"xyz"}"#.to_string(),
            is_json: true,
            ..Default::default()
        }
    }

    #[test]
    fn pre_request_sets_variable() {
        let result = run_pre_request("apt.setVariable('token', 'abc123')", &vars());
        assert!(result.error.is_none(), "{:?}", result.error);
        let set = result
            .effects
            .iter()
            .filter_map(|e| match e {
                SideEffect::SetVariable { name, value, .. } => Some((name.clone(), value.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(set.iter().any(|(n, v)| n == "token" && v == "abc123"));
    }

    #[test]
    fn variable_and_environment_setters_target_distinct_scopes() {
        let result = run_pre_request(
            "
                apt.variables.set('request-local', 'one');
                apt.setVariable('request-alias', 'two');
                apt.environment.set('environment-shared', 'three');
            ",
            &vars(),
        );

        assert!(result.error.is_none(), "{:?}", result.error);
        let scopes = result
            .effects
            .iter()
            .filter_map(|effect| match effect {
                SideEffect::SetVariable { scope, name, .. } => Some((name.as_str(), *scope)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(scopes.get("request-local"), Some(&VarScope::Request));
        assert_eq!(scopes.get("request-alias"), Some(&VarScope::Request));
        assert_eq!(
            scopes.get("environment-shared"),
            Some(&VarScope::Environment)
        );
    }

    #[test]
    fn get_variable_works() {
        let mut v = vars();
        v.insert("host".to_string(), "example.com".to_string());
        let result = run_pre_request("apt.echo(apt.getVariable('host'))", &v);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.logs.iter().any(|l| l.contains("example.com")));
    }

    #[test]
    fn set_variable_is_visible_immediately_in_same_script() {
        let result = run_pre_request(
            "apt.setVariable('a', '1'); apt.echo(apt.getVariable('a'))",
            &vars(),
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.logs.iter().any(|line| line == "1"));
    }

    #[test]
    fn assert_records_results() {
        let result = run_pre_request(
            "apt.assert(1 === 1, 'one'); apt.assert(1 === 2, 'two')",
            &vars(),
        );
        assert_eq!(result.assertions_passed, 1);
        assert_eq!(result.assertions_failed, 1);
    }

    #[test]
    fn post_request_reads_response() {
        let result = run_post_request(
            "if (response.json.code === 0) { apt.setVariable('token', response.json.token); } apt.assert(response.status === 200 && response.ok === true && response.error === undefined, 'ok response')",
            &vars(),
            &response(),
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.assertions_passed, 1);
        let token = result.effects.iter().find_map(|e| match e {
            SideEffect::SetVariable { name, value, .. } if name == "token" => Some(value.clone()),
            _ => None,
        });
        assert_eq!(token.as_deref(), Some("xyz"));
    }

    #[test]
    fn post_request_exposes_protocol_errors() {
        let response = HttpResponse {
            status: 200,
            error: Some("gRPC status 13: internal error".into()),
            ..Default::default()
        };
        let result = run_post_request(
            "apt.assert(response.ok === false, 'not ok'); apt.assert(response.error === 'gRPC status 13: internal error', 'error exposed')",
            &vars(),
            &response,
        );

        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.assertions_passed, 2);
        assert_eq!(result.assertions_failed, 0);
    }

    #[test]
    fn empty_script_is_noop() {
        let result = run_pre_request("   ", &vars());
        assert!(result.effects.is_empty());
        assert!(result.error.is_none());
    }

    #[test]
    fn syntax_error_is_captured() {
        let result = run_pre_request("apt.assert(", &vars());
        assert!(result.error.is_some());
    }

    #[test]
    fn infinite_loop_is_stopped_by_runtime_limit() {
        let result = run_pre_request("while (true) {}", &vars());

        let error = result.error.expect("loop limit should stop the script");
        assert!(
            error.to_ascii_lowercase().contains("loop")
                || error.to_ascii_lowercase().contains("iteration"),
            "{error}"
        );
    }
}
