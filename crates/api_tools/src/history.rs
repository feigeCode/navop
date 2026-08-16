use crate::request_store::RequestHistoryEntry;

pub const HISTORY_LIMIT: usize = 100;

pub fn push_history(history: &mut Vec<RequestHistoryEntry>, entry: RequestHistoryEntry) {
    history.insert(0, entry);
    history.truncate(HISTORY_LIMIT);
}

#[cfg(test)]
mod tests {
    use crate::http::RequestMethod;
    use crate::request_store::{RequestHistoryEntry, StoredRequest};

    use super::{HISTORY_LIMIT, push_history};

    fn entry(id: &str, sent_at: i64) -> RequestHistoryEntry {
        let mut request = StoredRequest::new(format!("Request {id}"), RequestMethod::Get);
        request.id = format!("request-{id}");
        RequestHistoryEntry {
            id: id.into(),
            sent_at,
            request_id: Some(request.id.clone()),
            request_name: request.name.clone(),
            method: request.method,
            url: format!("https://example.test/{id}"),
            status: 200,
            status_text: "OK".into(),
            time_ms: 12,
            size: 4,
            error: None,
            request,
        }
    }

    #[test]
    fn history_is_newest_first_and_bounded() {
        let mut history = (0..HISTORY_LIMIT)
            .map(|index| entry(&index.to_string(), index as i64))
            .collect::<Vec<_>>();

        push_history(&mut history, entry("new", 999));

        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].id, "new");
        assert_eq!(history.last().unwrap().id, "98");
    }

    #[test]
    fn history_keeps_a_complete_request_snapshot() {
        let mut history = Vec::new();
        let mut item = entry("snapshot", 1);
        item.request.body = "{\"saved\":true}".into();

        push_history(&mut history, item);

        assert_eq!(history[0].request.body, "{\"saved\":true}");
    }
}
