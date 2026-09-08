use futures_util::StreamExt;
use llm_connector::LlmClient;
use llm_connector::types::ResponsesRequest;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_invoke_responses_direct_success() {
    let server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "resp_direct_1",
        "object": "response",
        "created_at": 1710000000_u64,
        "model": "gpt-4.1",
        "status": "completed",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "Hello from responses"}
                ]
            }
        ],
        "usage": {
            "input_tokens": 3,
            "output_tokens": 4,
            "total_tokens": 7
        }
    });

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&server)
        .await;

    let client = LlmClient::openai("test-key", &server.uri()).expect("client should build");
    let request = ResponsesRequest {
        model: "gpt-4.1".to_string(),
        input: Some(serde_json::json!("hello")),
        ..Default::default()
    };

    let response = client
        .invoke_responses(&request)
        .await
        .expect("responses should succeed");

    assert_eq!(response.id, "resp_direct_1");
    assert_eq!(response.output_text, "Hello from responses");
    assert_eq!(
        response.usage.as_ref().and_then(|u| u.total_tokens),
        Some(7)
    );
}

#[tokio::test]
async fn test_invoke_responses_fallback_to_chat_on_404() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": {
                "message": "responses endpoint not found"
            }
        })))
        .mount(&server)
        .await;

    let chat_body = serde_json::json!({
        "id": "chatcmpl_1",
        "object": "chat.completion",
        "created": 1710000001_u64,
        "model": "gpt-4.1",
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": "Hello from fallback"},
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 3,
            "completion_tokens": 4,
            "total_tokens": 7
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body))
        .mount(&server)
        .await;

    let client = LlmClient::openai("test-key", &server.uri()).expect("client should build");
    let request = ResponsesRequest {
        model: "gpt-4.1".to_string(),
        input: Some(serde_json::json!("hello")),
        ..Default::default()
    };

    let response = client
        .invoke_responses(&request)
        .await
        .expect("fallback should succeed");

    assert_eq!(response.object, "response");
    assert_eq!(response.output_text, "Hello from fallback");
    assert_eq!(
        response.usage.as_ref().and_then(|u| u.total_tokens),
        Some(7)
    );
}

#[tokio::test]
async fn test_invoke_responses_stream_fallback_event_ordering() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": {
                "message": "responses endpoint not found"
            }
        })))
        .mount(&server)
        .await;

    let sse_body = concat!(
        "data: {\"id\":\"chatcmpl_stream_1\",\"object\":\"chat.completion.chunk\",\"created\":1710000010,\"model\":\"gpt-4.1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello \"}}]}\n\n",
        "data: {\"id\":\"chatcmpl_stream_1\",\"object\":\"chat.completion.chunk\",\"created\":1710000011,\"model\":\"gpt-4.1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"World\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let client = LlmClient::openai("test-key", &server.uri()).expect("client should build");
    let request = ResponsesRequest {
        model: "gpt-4.1".to_string(),
        input: Some(serde_json::json!("hello")),
        stream: Some(true),
        ..Default::default()
    };

    let mut stream = client
        .invoke_responses_stream(&request)
        .await
        .expect("fallback stream should succeed");

    let mut event_types = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.expect("stream event should parse");
        event_types.push(event.event_type);
    }

    assert!(event_types.len() >= 3);
    assert_eq!(event_types[0], "response.created");
    assert!(
        event_types
            .iter()
            .any(|e| e == "response.output_text.delta")
    );
    assert_eq!(
        event_types.last().map(String::as_str),
        Some("response.completed")
    );
}

#[tokio::test]
async fn test_invoke_responses_stream_direct_success() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream_1\",\"object\":\"response\",\"status\":\"in_progress\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_stream_1\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream_1\",\"object\":\"response\",\"status\":\"completed\"}}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let client = LlmClient::openai("test-key", &server.uri()).expect("client should build");
    let request = ResponsesRequest {
        model: "gpt-4.1".to_string(),
        input: Some(serde_json::json!("hello")),
        stream: Some(true),
        ..Default::default()
    };

    let mut stream = client
        .invoke_responses_stream(&request)
        .await
        .expect("direct stream should succeed");

    let mut event_types = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.expect("stream event should parse");
        event_types.push(event.event_type);
    }

    assert_eq!(
        event_types,
        vec![
            "response.created".to_string(),
            "response.output_text.delta".to_string(),
            "response.completed".to_string()
        ]
    );
}
