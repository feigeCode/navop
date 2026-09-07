//! `navop.dev` host 模块：开发者工具扩展专用。
//!
//! 仅当 shell view 声明 `modules: [..., "dev"]` 时装配。操作集由 main
//! 层启动时通过 [`set_dev_host_ops`] 注入(Global);未注入时模块为空壳
//!（fail-closed）。避免 universal-plugins 反向依赖 main 的
//! DevExtensionRegistry。

use std::rc::Rc;

use gpui_shell::{HostError, HostModule, HostValue};

/// 宿主注入的 dev 操作集。
pub struct DevHostOps {
    /// 列出 dev 工程:[{ root, id, name, version, error?, views:[{id,title,surface,category?}] }]。
    pub list: Rc<dyn Fn() -> Result<HostValue, HostError>>,
    /// 加载/重载工程目录,返回 { id, error? }。
    pub open: Rc<dyn Fn(&str) -> Result<HostValue, HostError>>,
    /// 移除工程注册,返回 null。
    pub remove: Rc<dyn Fn(&str) -> Result<HostValue, HostError>>,
    /// 打开 dev 扩展视图(需要 window),返回 null。
    pub open_view: Rc<dyn Fn(&str, &str) -> Result<HostValue, HostError>>,
    /// 读取 dev 工程日志尾部,返回 string[]。
    pub logs: Rc<dyn Fn(&str, f64) -> Result<HostValue, HostError>>,
}

impl gpui::Global for GlobalDevHostOps {}

/// dev 操作集 Global。
pub struct GlobalDevHostOps {
    pub ops: Rc<DevHostOps>,
}

/// main 启动时注入 dev 操作集。
pub fn set_dev_host_ops(ops: Rc<DevHostOps>, cx: &mut gpui::App) {
    cx.set_global(GlobalDevHostOps { ops });
}

/// 装配 navop.dev 模块;操作集缺失时返回空模块(所有调用 MethodNotFound)。
pub(super) fn dev_module() -> HostModule {
    let Some(ops) = gpui_shell::with_current_app(|cx| {
        cx.try_global::<GlobalDevHostOps>()
            .map(|global| Rc::clone(&global.ops))
    })
    .flatten()
    else {
        return HostModule::new("navop.dev");
    };
    let list = Rc::clone(&ops.list);
    let open = Rc::clone(&ops.open);
    let remove = Rc::clone(&ops.remove);
    let open_view = Rc::clone(&ops.open_view);
    let logs = Rc::clone(&ops.logs);
    HostModule::new("navop.dev")
        .declarations(
            r#"
            export interface DevViewInfo { id: string; title: string; surface: string; category?: string; }
            export interface DevProjectInfo {
              root: string;
              id: string;
              name: string;
              version: string;
              error?: string;
              views: DevViewInfo[];
            }
            export function list(): DevProjectInfo[];
            export function open(rootDir: string): { id: string; error?: string };
            export function remove(rootDir: string): void;
            export function openView(extensionId: string, viewId: string): void;
            export function logs(rootDir: string, tail?: number): string[];
            "#,
        )
        .function("list", move |_| list())
        .function("open", move |arguments| {
            let root = arguments.string(0)?;
            open(&root)
        })
        .function("remove", move |arguments| {
            let root = arguments.string(0)?;
            remove(&root)
        })
        .function("openView", move |arguments| {
            let extension_id = arguments.string(0)?;
            let view_id = arguments.string(1)?;
            open_view(&extension_id, &view_id)
        })
        .function("logs", move |arguments| {
            let root = arguments.string(0)?;
            let tail = arguments
                .get(1)
                .map(|_| arguments.number(1))
                .transpose()?
                .unwrap_or(200.0);
            logs(&root, tail)
        })
}
