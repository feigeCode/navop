use agent_runtime::model::ToolCall;
use one_core::llm::StreamingResponse;

/// 把流式分片的工具调用合并进累积器。
///
/// 这里同时兼容两种上游形态:
/// - 完整快照:同一调用反复带着 id/type/name 与递增 arguments,用最新快照覆盖;
/// - 参数 delta:首包带 id/type/name,后续包只有 arguments 碎片,需要合并到已有调用。
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
            Some(existing) => merge_existing_tool_call(existing, call),
            None => acc.push(call.clone()),
        }
    }
}

fn merge_existing_tool_call(existing: &mut ToolCall, call: &ToolCall) {
    if is_argument_delta(call) {
        if existing.function.arguments == "{}" {
            existing.function.arguments.clear();
        }
        existing.merge_delta(call);
    } else {
        *existing = call.clone();
    }
}

/// 参数增量分片只带 arguments。
///
/// 判定只看身份字段 `function.name`：工具名只在首个分片出现，后续分片必然为空。
/// 不能用 `call_type` 判定——它是常量 `"function"`，部分适配器（`/responses` 的
/// `response.function_call_arguments.delta`）会在参数分片里照样填上，一旦纳入
/// 判定，整条调用就会被当成新快照覆盖，已捕获的 id/name 全部丢失。
fn is_argument_delta(call: &ToolCall) -> bool {
    call.function.name.is_empty()
}

/// 判断两个流式工具调用分片是否指向同一次调用:优先比 `index`,否则比非空 `id`。
fn same_tool_call(a: &ToolCall, b: &ToolCall) -> bool {
    match (a.index, b.index) {
        (Some(x), Some(y)) => x == y,
        _ => !a.id.is_empty() && a.id == b.id,
    }
}
