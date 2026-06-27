use agent_runtime::model::ToolCall;
use one_core::llm::StreamingResponse;

/// 把流式分片的工具调用合并进累积器。
///
/// **关键**:`llm-connector`(`sse.rs::accumulate_tool_calls`)已在连接器侧按 `index`
/// 累积工具调用,并在**每个携带 `tool_calls` 的分片上重发当前完整快照**(`arguments`
/// 随分片增长,且只含 `is_complete` 的调用)。因此这里**绝不能再次追加 `arguments`**——
/// 否则会把同一次调用的多个"增长前缀"误当成多次独立调用,而前缀都是不完整 JSON,
/// 后续 `serde_json::from_str` 会报 `EOF while parsing an object`(计划工具因此频繁失败、
/// 多步任务中途停止)。正确做法:按 `index`(回退到非空 `id`)用最新快照**覆盖**。
pub(super) fn merge_stream_tool_calls(acc: &mut Vec<ToolCall>, chunk: &StreamingResponse) {
    let Some(snapshot) = chunk
        .choices
        .first()
        .and_then(|c| c.delta.tool_calls.as_ref())
    else {
        return;
    };
    for call in snapshot {
        match acc
            .iter_mut()
            .find(|existing| same_tool_call(existing, call))
        {
            Some(existing) => *existing = call.clone(),
            None => acc.push(call.clone()),
        }
    }
}

/// 判断两个流式工具调用分片是否指向同一次调用:优先比 `index`,否则比非空 `id`。
fn same_tool_call(a: &ToolCall, b: &ToolCall) -> bool {
    match (a.index, b.index) {
        (Some(x), Some(y)) => x == y,
        _ => !a.id.is_empty() && a.id == b.id,
    }
}
