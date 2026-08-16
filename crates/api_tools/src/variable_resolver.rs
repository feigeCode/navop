use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct VariableResolver<'a> {
    vars: &'a BTreeMap<String, String>,
    dynamic: BTreeMap<String, String>,
}

impl<'a> VariableResolver<'a> {
    pub(crate) fn new(vars: &'a BTreeMap<String, String>) -> Self {
        Self {
            vars,
            dynamic: BTreeMap::new(),
        }
    }

    pub(crate) fn substitute(&mut self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let bytes = input.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index..].starts_with(b"{{")
                && let Some(end) = find_placeholder_end(&bytes[index + 2..])
            {
                let name = input[index + 2..index + 2 + end].trim();
                if let Some(value) = self.resolve(name) {
                    out.push_str(&value);
                } else {
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                }
                index += end + 4;
                continue;
            }
            let ch = input[index..]
                .chars()
                .next()
                .expect("index must remain on a UTF-8 boundary");
            out.push(ch);
            index += ch.len_utf8();
        }
        out
    }

    fn resolve(&mut self, name: &str) -> Option<String> {
        self.vars
            .get(name)
            .cloned()
            .or_else(|| self.dynamic.get(name).cloned())
            .or_else(|| {
                let value = dynamic_variable(name)?;
                self.dynamic.insert(name.to_string(), value.clone());
                Some(value)
            })
    }
}

fn find_placeholder_end(input: &[u8]) -> Option<usize> {
    input.windows(2).position(|pair| pair == b"}}")
}

fn dynamic_variable(name: &str) -> Option<String> {
    match name {
        "$random" => Some(random_alphanumeric(10)),
        "$uuid" => Some(uuid::Uuid::new_v4().to_string()),
        "$sparkid" => Some(uuid::Uuid::new_v4().simple().to_string()),
        "$timestamp" => Some(unix_timestamp()),
        _ => None,
    }
}

fn unix_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn random_alphanumeric(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    (0..len)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut mixed = state;
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            mixed ^= mixed >> 31;
            ALPHABET[(mixed % ALPHABET.len() as u64) as usize] as char
        })
        .collect()
}
