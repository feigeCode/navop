//! 导入窗口里统一套用到迁移连接上的高级设置默认项。
//!
//! 迁移场景的源端设备（防火墙、交换机、工控机）往往只支持旧版密钥交换与主机密钥算法，
//! 几百条连接逐条去高级设置里勾选并不现实，所以在导入阶段一次性指定。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImportConnectionDefaults {
    /// 允许旧版 SSH 算法（ssh-dss 等）。
    pub(crate) allow_legacy_algorithms: bool,
    /// 禁用 Shell 集成探测与注入。
    pub(crate) disable_shell_integration: bool,
}

impl Default for ImportConnectionDefaults {
    /// 迁移场景默认放开旧版算法：它只是允许协商，现代服务器仍会优先选新算法，
    /// 但老设备不加这个开关会直接连不上。Shell 集成保持开启，只对需要的设备手动关闭。
    fn default() -> Self {
        Self {
            allow_legacy_algorithms: true,
            disable_shell_integration: false,
        }
    }
}

impl ImportConnectionDefaults {
    /// 转换成 `SshParams` 里 `Option<bool>` 的写法：`false` 表示不写该字段（沿用全局默认）。
    pub(crate) fn allow_legacy_algorithms_param(self) -> Option<bool> {
        self.allow_legacy_algorithms.then_some(true)
    }

    pub(crate) fn disable_shell_integration_param(self) -> Option<bool> {
        self.disable_shell_integration.then_some(true)
    }
}
