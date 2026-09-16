use std::collections::HashMap;

/// Parsed representation of the `X-ClickHouse-Summary` HTTP response header.
///
/// Provides typed getters for known fields and a generic [`get`](Self::get)
/// fallback for forward-compatibility with future ClickHouse versions.
///
/// All getters return `Option<u64>`: `None` if the field is absent or cannot
/// be parsed. This ensures deserialization never fails, even if ClickHouse
/// renames, removes, or adds fields.
///
/// Note: the summary values may be incomplete unless the query was executed
/// with `wait_end_of_query=1`, because ClickHouse sends this header before
/// the response body and the values reflect progress at that point.
#[derive(Debug, Clone)]
pub struct QuerySummary {
    fields: HashMap<String, String>,
}

impl QuerySummary {
    /// Returns the raw string value for the given key, if present.
    ///
    /// Use this to access fields that are not yet covered by typed getters,
    /// e.g. fields added in newer ClickHouse versions.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn read_rows(&self) -> Option<u64> {
        self.get_u64("read_rows")
    }

    pub fn read_bytes(&self) -> Option<u64> {
        self.get_u64("read_bytes")
    }

    pub fn written_rows(&self) -> Option<u64> {
        self.get_u64("written_rows")
    }

    pub fn written_bytes(&self) -> Option<u64> {
        self.get_u64("written_bytes")
    }

    pub fn total_rows_to_read(&self) -> Option<u64> {
        self.get_u64("total_rows_to_read")
    }

    pub fn result_rows(&self) -> Option<u64> {
        self.get_u64("result_rows")
    }

    pub fn result_bytes(&self) -> Option<u64> {
        self.get_u64("result_bytes")
    }

    pub fn elapsed_ns(&self) -> Option<u64> {
        self.get_u64("elapsed_ns")
    }

    pub fn memory_usage(&self) -> Option<u64> {
        self.get_u64("memory_usage")
    }

    fn get_u64(&self, key: &str) -> Option<u64> {
        self.fields.get(key)?.parse().ok()
    }

    /// Parses the raw header value into a `QuerySummary`.
    ///
    /// Returns `None` if the value is not valid JSON or not an object with
    /// string values. This matches ClickHouse's encoding, where all values
    /// are JSON strings (e.g. `"1000"` instead of `1000`).
    pub(crate) fn from_header(raw: &str) -> Option<Self> {
        let fields: HashMap<String, String> = serde_json::from_str(raw).ok()?;
        Some(Self { fields })
    }
}
