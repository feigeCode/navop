//! `navop.dev` 宿主能力：开发者工具扩展专用。
//!
//! 只有 view 声明 `modules: [..., "dev"]` 时才装配。操作集由 main 层在启动
//! 时注入（[`set_dev_ops`]），未注入时模块是空壳（fail-closed）——这样
//! universal-plugins 不必反向依赖 main 的 `DevExtensionRegistry`。
//!
//! 每个操作都在**主线程**上执行：它们要读写 GPUI 全局、开窗口、弹文件
//! 选择器。引擎线程上的 handler 只负责把参数搬过去（见 [`super::dispatch`]）。

use std::rc::Rc;

use gpui::App;
use serde_json::Value;

/// 宿主注入的 dev 操作集。
///
/// 参数与返回值都是 JSON：这一层不认识 HostValue，也不需要认识 ——
/// 它就是"主线程上的一组函数"，由页面通过 `navop.dev.*` 调用。
#[derive(Default)]
pub struct DevOps {
    /// 列出已加载的 dev 工程。
    pub list: Option<Rc<dyn Fn(&mut App) -> Result<Value, String>>>,
    /// 加载/重载一个工程目录。
    pub open: Option<Rc<dyn Fn(&mut App, &str) -> Result<Value, String>>>,
    /// 重读 manifest 并重建 catalog。
    pub reload: Option<Rc<dyn Fn(&mut App, &str) -> Result<Value, String>>>,
    /// 开关对该工程目录的变更轮询。
    pub watch: Option<Rc<dyn Fn(&mut App, &str) -> Result<Value, String>>>,
    /// 从注册表移除工程。
    pub remove: Option<Rc<dyn Fn(&mut App, &str) -> Result<Value, String>>>,
    /// 打开工程的某个 shell view。
    pub open_view: Option<Rc<dyn Fn(&mut App, &str, &str) -> Result<Value, String>>>,
    /// 读工程日志尾部。
    pub logs: Option<Rc<dyn Fn(&mut App, &str, usize) -> Result<Value, String>>>,
    /// 弹原生目录选择器（返回 "pending"）。
    pub pick_start: Option<Rc<dyn Fn(&mut App) -> Result<Value, String>>>,
    /// 取目录选择结果（未完成时为 null）。
    pub pick_result: Option<Rc<dyn Fn(&mut App) -> Result<Value, String>>>,
}

impl gpui::Global for GlobalDevOps {}

/// dev 操作集 Global。
pub struct GlobalDevOps {
    pub ops: Rc<DevOps>,
}

/// main 启动时注入 dev 操作集。
pub fn set_dev_ops(ops: Rc<DevOps>, cx: &mut App) {
    cx.set_global(GlobalDevOps { ops });
}

/// 取当前注入的操作集。未注入时 `None`（模块缺席，不是报错）。
pub(crate) fn ops(cx: &App) -> Option<Rc<DevOps>> {
    cx.try_global::<GlobalDevOps>()
        .map(|global| Rc::clone(&global.ops))
}

/// 取某一个操作；未注入时报一句能看懂的错。
pub(crate) fn op<T>(cx: &App, pick: impl FnOnce(&DevOps) -> Option<T>) -> Result<T, String> {
    let ops = ops(cx).ok_or_else(|| "dev 能力尚未注入".to_string())?;
    pick(&ops).ok_or_else(|| "这个 dev 操作没有接上".to_string())
}
