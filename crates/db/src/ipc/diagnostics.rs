//! 外部 IPC 驱动结果集的“二进制化”诊断。
//!
//! 与 [`crate::result_diagnostics`] 同一套排查目标：确认“文本列被显示成二进制”是驱动下发了
//! bytes cell，还是宿主解码误判。驱动侧历史上曾按 Go 运行时类型判定 MySQL 协议文本列，因此
//! 这里必须同时打印驱动 id/版本与列 spec。定位完成后可整体删除。

use crate::result_diagnostics::{
    BinarySample, DIAG_PREFIX, MAX_BINARY_SAMPLES, MAX_DIAGNOSTIC_COLUMNS, render_limited,
    render_samples,
};
use db_value::{CellState, DbValue, ResultRow};
use extension_protocol::row::ColumnSpec;
use tracing::{debug, info};

/// 一次 IPC 结果里的 bytes 单元格统计。
pub(crate) struct IpcBinaryCells {
    pub total: usize,
    pub samples: Vec<BinarySample>,
}

/// 统计 IPC 结果里被判为二进制的单元格，并收集有界样本。
pub(crate) fn ipc_binary_cells(specs: &[ColumnSpec], rows: &[ResultRow]) -> IpcBinaryCells {
    let mut total = 0usize;
    let mut samples = Vec::new();
    for row in rows {
        for (column_index, cell) in row.cells.iter().enumerate() {
            let CellState::Decoded(DbValue::Binary(bytes)) = cell else {
                continue;
            };
            total += 1;
            if samples.len() < MAX_BINARY_SAMPLES {
                let label = specs
                    .get(column_index)
                    .map(|spec| spec.name.as_str())
                    .unwrap_or("<unknown>");
                samples.push(BinarySample::new(row.id, label, bytes));
            }
        }
    }
    IpcBinaryCells { total, samples }
}

fn render_columns(specs: &[ColumnSpec]) -> String {
    render_limited(specs, MAX_DIAGNOSTIC_COLUMNS, |spec| {
        format!(
            "{}[type={},type_kind={:?},charset={}]",
            spec.name,
            spec.type_str,
            spec.type_kind,
            spec.extra
                .get("result_charset")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown"),
        )
    })
}

/// 输出一次 IPC 驱动的结果集列 spec 与 bytes 单元格样本。
///
/// 只有真的出现 bytes cell 时才用 `info!`，普通查询降到 `debug!`。
pub(crate) fn log_ipc_result(driver: &str, specs: &[ColumnSpec], cells: &IpcBinaryCells) {
    if cells.total == 0 {
        debug!(
            "{DIAG_PREFIX}[ipc] driver={driver} result columns({}): {}",
            specs.len(),
            render_columns(specs)
        );
        return;
    }
    info!(
        "{DIAG_PREFIX}[ipc] driver={driver} bytes cells={} in columns({}): {}",
        cells.total,
        specs.len(),
        render_columns(specs)
    );
    info!(
        "{DIAG_PREFIX}[ipc] driver={driver} binary samples({} of {}): {}",
        cells.samples.len(),
        cells.total,
        render_samples(&cells.samples)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::row::ColumnTypeKind;

    fn specs() -> Vec<ColumnSpec> {
        vec![
            ColumnSpec::new("id", "BIGINT", ColumnTypeKind::I64),
            ColumnSpec::new("name", "VARCHAR", ColumnTypeKind::Text),
        ]
    }

    fn rows(count: u64) -> Vec<ResultRow> {
        (0..count)
            .map(|index| ResultRow {
                id: index,
                cells: vec![
                    CellState::Decoded(DbValue::Integer(index.to_string())),
                    CellState::Decoded(DbValue::Binary(b"abc".to_vec())),
                ],
            })
            .collect()
    }

    #[test]
    fn binary_cells_count_all_rows_but_sample_only_a_bounded_number() {
        let cells = ipc_binary_cells(&specs(), &rows(8));

        assert_eq!(cells.total, 8);
        assert_eq!(cells.samples.len(), MAX_BINARY_SAMPLES);
        assert_eq!(cells.samples[0].column, "name");
        assert!(cells.samples[0].render().contains("hex=616263"));
    }

    #[test]
    fn text_only_results_report_no_bytes_cells() {
        let text_rows = vec![ResultRow {
            id: 0,
            cells: vec![
                CellState::Decoded(DbValue::Integer("1".to_string())),
                CellState::Decoded(DbValue::Text("小明".to_string())),
            ],
        }];

        assert_eq!(ipc_binary_cells(&specs(), &text_rows).total, 0);
    }

    #[test]
    fn unknown_columns_fall_back_to_a_placeholder_label() {
        let cells = ipc_binary_cells(&[], &rows(1));

        assert_eq!(cells.samples[0].column, "<unknown>");
    }
}
