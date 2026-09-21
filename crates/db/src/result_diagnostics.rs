//! “文本结果被展示成 `二进制 · N B`”问题的有界诊断日志。
//!
//! 触发场景：部分用户的 VARCHAR/TEXT 结果在表格里显示为二进制。可能的根因分三类，只有
//! “列的 wire 元数据 + 实际字节”能区分：
//!
//! 1. 服务端把结果按字节下发（会话 `character_set_results=binary`，或兼容引擎/代理固定
//!    给出 collation 63）；
//! 2. 该列本身就是 BLOB/VARBINARY（展示正确，只是与用户预期不符）；
//! 3. 列是字符族，但字节不符合该列 charset，解码失败后降级成二进制 sidecar。
//!
//! 输出统一带 [`DIAG_PREFIX`] 前缀，排查时直接 grep 该前缀。所有行都有条数上限，普通查询
//! （没有二进制值）只走 `debug!`，问题查询才用 `info!`，因此默认 `info` 级别的日志文件里
//! 出现的诊断行就是有效证据。定位完成后本模块与各调用点可整体删除。

use tracing::{debug, info};

/// 诊断行的统一前缀。
pub(crate) const DIAG_PREFIX: &str = "[result-diag]";

/// 单个结果集最多打印多少列的元数据。
pub(crate) const MAX_DIAGNOSTIC_COLUMNS: usize = 40;
/// 单个结果集最多打印多少个二进制单元格样本。
pub(crate) const MAX_BINARY_SAMPLES: usize = 5;
/// 每个样本最多保留多少字节的 hex 预览。
pub(crate) const MAX_PREVIEW_BYTES: usize = 16;

/// 一列的 wire 元数据诊断（MySQL 内置驱动）。
pub(crate) struct ColumnDiagnostic {
    pub label: String,
    pub native_type: String,
    /// `mysql_common::consts::ColumnFlags` 的原始位（底层 `u16`）。
    pub flags: u16,
    pub collation_id: u16,
    pub charset: Option<String>,
    pub collation: Option<String>,
    /// `mysql::codec::describe_binary_cause` 的分类结果。
    pub cause: &'static str,
}

impl ColumnDiagnostic {
    fn render(&self) -> String {
        format!(
            "{}[type={},flags=0x{:04X},collation_id={},charset={},collation={},cause={}]",
            self.label,
            self.native_type,
            self.flags,
            self.collation_id,
            self.charset.as_deref().unwrap_or("unknown"),
            self.collation.as_deref().unwrap_or("unknown"),
            self.cause,
        )
    }
}

/// 一个被判为二进制的单元格样本，只保留有界预览。
pub(crate) struct BinarySample {
    pub row: u64,
    pub column: String,
    pub total_len: usize,
    utf8: bool,
    control_bytes: usize,
    preview: Vec<u8>,
}

impl BinarySample {
    pub(crate) fn new(row: u64, column: &str, bytes: &[u8]) -> Self {
        Self {
            row,
            column: column.to_string(),
            total_len: bytes.len(),
            utf8: std::str::from_utf8(bytes).is_ok(),
            control_bytes: bytes
                .iter()
                .copied()
                .filter(|byte| is_diagnostic_control_byte(*byte))
                .count(),
            preview: bytes.iter().copied().take(MAX_PREVIEW_BYTES).collect(),
        }
    }

    pub(crate) fn render(&self) -> String {
        let hex: String = self
            .preview
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        format!(
            "row={},col={},len={},utf8={},control_bytes={},hex={}",
            self.row,
            self.column,
            self.total_len,
            if self.utf8 { "yes" } else { "no" },
            self.control_bytes,
            hex,
        )
    }
}

fn is_diagnostic_control_byte(byte: u8) -> bool {
    byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r' | b'\t')
}

pub(crate) fn render_limited<T>(
    items: &[T],
    limit: usize,
    render: impl Fn(&T) -> String,
) -> String {
    let rendered = items
        .iter()
        .take(limit)
        .map(render)
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        format!("{rendered}, …(+{} more)", items.len() - limit)
    } else {
        rendered
    }
}

pub(crate) fn render_samples(samples: &[BinarySample]) -> String {
    render_limited(samples, MAX_BINARY_SAMPLES, BinarySample::render)
}

/// 输出一次内置 MySQL 结果集的列元数据与二进制样本。
pub(crate) fn log_mysql_result(
    columns: &[ColumnDiagnostic],
    binary_cells: usize,
    samples: &[BinarySample],
) {
    fn render_columns(columns: &[ColumnDiagnostic]) -> String {
        render_limited(columns, MAX_DIAGNOSTIC_COLUMNS, ColumnDiagnostic::render)
    }

    if binary_cells == 0 {
        debug!(
            "{DIAG_PREFIX}[mysql] result columns({}): {}",
            columns.len(),
            render_columns(columns)
        );
        return;
    }
    info!(
        "{DIAG_PREFIX}[mysql] binary cells={} in columns({}): {}",
        binary_cells,
        columns.len(),
        render_columns(columns)
    );
    info!(
        "{DIAG_PREFIX}[mysql] binary samples({} of {}): {}",
        samples.len(),
        binary_cells,
        render_samples(samples)
    );
}

/// 直接表查询 schema 纠偏（`query_result_normalization`）的一列决策。
pub(crate) struct ReconciliationDiagnostic {
    pub label: String,
    pub result_charset: Option<String>,
    pub schema_type: String,
    pub schema_charset: Option<String>,
    /// 选中用来把字节解回文本的 charset；`None` 表示这列保持二进制。
    pub decoder: Option<String>,
}

impl ReconciliationDiagnostic {
    fn render(&self) -> String {
        format!(
            "{}[result_charset={},schema_type={},schema_charset={},decoder={}]",
            self.label,
            self.result_charset.as_deref().unwrap_or("unknown"),
            self.schema_type,
            self.schema_charset.as_deref().unwrap_or("unknown"),
            self.decoder.as_deref().unwrap_or("none"),
        )
    }
}

/// 输出一次表查询纠偏的输入：说明每列为什么（没有）被解回文本。
pub(crate) fn log_mysql_reconciliation(columns: &[ReconciliationDiagnostic], binary_cells: usize) {
    info!(
        "{DIAG_PREFIX}[mysql] direct table query schema reconciliation, binary cells={} in columns({}): {}",
        binary_cells,
        columns.len(),
        render_limited(
            columns,
            MAX_DIAGNOSTIC_COLUMNS,
            ReconciliationDiagnostic::render
        )
    );
}

/// 直接表查询没有二进制单元格时的 debug 级提示。
pub(crate) fn log_mysql_reconciliation_without_binary_cells(columns: usize) {
    debug!("{DIAG_PREFIX}[mysql] direct table query without binary cells, columns({columns})");
}

/// 输出表查询纠偏的结果：还剩多少二进制单元格没有被解回文本。
pub(crate) fn log_mysql_reconciliation_outcome(binary_before: usize, binary_after: usize) {
    info!(
        "{DIAG_PREFIX}[mysql] direct table query schema reconciliation result: binary cells {binary_before} -> {binary_after}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mysql_column(charset: Option<&str>) -> ColumnDiagnostic {
        ColumnDiagnostic {
            label: "excel_user_name".to_string(),
            native_type: "VAR_STRING".to_string(),
            flags: 0x80,
            collation_id: 63,
            charset: charset.map(str::to_string),
            collation: None,
            cause: "wire-binary",
        }
    }

    #[test]
    fn column_diagnostic_keeps_raw_collation_id_and_unknown_names() {
        let rendered = mysql_column(Some("binary")).render();

        assert_eq!(
            rendered,
            "excel_user_name[type=VAR_STRING,flags=0x0080,collation_id=63,charset=binary,collation=unknown,cause=wire-binary]"
        );
        assert!(mysql_column(None).render().contains("charset=unknown"));
    }

    #[test]
    fn binary_sample_reports_length_encoding_and_bounded_hex() {
        let bytes = "中文".as_bytes().to_vec();
        let sample = BinarySample::new(2, "excel_user_name", &bytes).render();

        assert_eq!(
            sample,
            "row=2,col=excel_user_name,len=6,utf8=yes,control_bytes=0,hex=E4B8ADE69687"
        );
    }

    #[test]
    fn binary_sample_truncates_preview_but_keeps_real_length() {
        let bytes = vec![0x41u8; MAX_PREVIEW_BYTES + 8];
        let sample = BinarySample::new(0, "blob", &bytes);

        assert_eq!(sample.total_len, MAX_PREVIEW_BYTES + 8);
        assert_eq!(sample.preview.len(), MAX_PREVIEW_BYTES);
        let rendered = sample.render();
        assert!(rendered.contains(&format!("len={}", MAX_PREVIEW_BYTES + 8)));
    }

    #[test]
    fn binary_sample_counts_control_bytes_but_ignores_whitespace() {
        let sample = BinarySample::new(0, "blob", &[0x00, b'\n', b'\t', 0x1F]);

        assert_eq!(sample.control_bytes, 2);
    }

    #[test]
    fn rendering_limits_long_column_lists() {
        let columns = (0..MAX_DIAGNOSTIC_COLUMNS + 3)
            .map(|_| mysql_column(None))
            .collect::<Vec<_>>();
        let rendered = render_limited(&columns, MAX_DIAGNOSTIC_COLUMNS, ColumnDiagnostic::render);

        assert!(rendered.ends_with("…(+3 more)"));
        assert_eq!(
            rendered.matches("excel_user_name[").count(),
            MAX_DIAGNOSTIC_COLUMNS
        );
    }

    #[test]
    fn reconciliation_diagnostic_marks_columns_left_as_binary() {
        let kept = ReconciliationDiagnostic {
            label: "payload".to_string(),
            result_charset: Some("binary".to_string()),
            schema_type: "blob".to_string(),
            schema_charset: Some("binary".to_string()),
            decoder: None,
        };
        let decoded = ReconciliationDiagnostic {
            label: "excel_user_name".to_string(),
            result_charset: Some("binary".to_string()),
            schema_type: "varchar".to_string(),
            schema_charset: Some("utf8mb4".to_string()),
            decoder: Some("utf8mb4".to_string()),
        };

        assert_eq!(
            kept.render(),
            "payload[result_charset=binary,schema_type=blob,schema_charset=binary,decoder=none]"
        );
        assert!(decoded.render().ends_with("decoder=utf8mb4]"));
    }
}
