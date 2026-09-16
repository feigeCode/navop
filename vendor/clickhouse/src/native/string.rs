use bytes::Bytes;
use std::cmp::Ordering;
use std::convert::Infallible;
use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Clone, PartialEq, Eq)]
pub struct MaybeUtf8 {
    bytes: Bytes,
    /// SAFETY: must not change for the lifetime of this object, must be valid UTF-8 on construction
    is_utf8: bool,
}

impl MaybeUtf8 {
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self::from(bytes.into())
    }

    pub fn from_string(string: impl Into<String>) -> Self {
        Self::from(string.into())
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

impl From<&'static [u8]> for MaybeUtf8 {
    fn from(value: &'static [u8]) -> Self {
        Self::from_bytes(value)
    }
}

impl From<Vec<u8>> for MaybeUtf8 {
    fn from(value: Vec<u8>) -> Self {
        Self::from_bytes(value)
    }
}

impl From<Bytes> for MaybeUtf8 {
    fn from(bytes: Bytes) -> Self {
        Self {
            is_utf8: str::from_utf8(&bytes).is_ok(),
            bytes,
        }
    }
}

impl From<String> for MaybeUtf8 {
    fn from(value: String) -> Self {
        Self {
            bytes: value.into(),
            is_utf8: true,
        }
    }
}

impl From<&'static str> for MaybeUtf8 {
    fn from(value: &'static str) -> Self {
        Self {
            bytes: value.into(),
            is_utf8: true,
        }
    }
}

impl FromStr for MaybeUtf8 {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_string(s))
    }
}

impl MaybeUtf8 {
    pub fn is_utf8(&self) -> bool {
        self.is_utf8
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_str(&self) -> Option<&str> {
        // SAFETY: UTF-8 validity is checked on construction
        self.is_utf8
            .then(|| unsafe { str::from_utf8_unchecked(&self.bytes) })
    }
}

impl Hash for MaybeUtf8 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Produce the same hash as `&[u8]` or `&str`;
        // `impl Hash for str` adds a domain-separation postfix so its routine is separate.
        match self.as_str() {
            Some(s) => s.hash(state),
            None => self.bytes.hash(state),
        }
    }
}

/// Compares lexicographically regardless of UTF-8-ness.
impl PartialOrd for MaybeUtf8 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd<str> for MaybeUtf8 {
    fn partial_cmp(&self, other: &str) -> Option<Ordering> {
        Some(self.as_bytes().cmp(other.as_bytes()))
    }
}

impl PartialOrd<[u8]> for MaybeUtf8 {
    fn partial_cmp(&self, other: &[u8]) -> Option<Ordering> {
        Some(self.as_bytes().cmp(other))
    }
}

impl Ord for MaybeUtf8 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialEq<str> for MaybeUtf8 {
    fn eq(&self, other: &str) -> bool {
        self.as_str().is_some_and(|s| s == other)
    }
}

impl PartialEq<&str> for MaybeUtf8 {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
    }
}

impl PartialEq<[u8]> for MaybeUtf8 {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes().eq(other)
    }
}

impl PartialEq<&[u8]> for MaybeUtf8 {
    fn eq(&self, other: &&[u8]) -> bool {
        *self == **other
    }
}

impl Debug for MaybeUtf8 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.as_str() {
            Some(s) => Debug::fmt(s, f),
            None => write!(f, "\"{}\"", self.bytes.escape_ascii()),
        }
    }
}

impl Display for MaybeUtf8 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.as_str() {
            Some(s) => Display::fmt(s, f),
            None => {
                for chunk in self.bytes.utf8_chunks() {
                    write!(f, "{}", chunk.valid())?;

                    for b in chunk.invalid() {
                        write!(f, "{}", b.escape_ascii())?;
                    }
                }

                Ok(())
            }
        }
    }
}

impl hashbrown::Equivalent<MaybeUtf8> for str {
    fn equivalent(&self, key: &MaybeUtf8) -> bool {
        key == self
    }
}

#[cfg(test)]
mod tests {
    use super::MaybeUtf8;
    use std::hash::{BuildHasher, RandomState};

    #[test]
    fn is_utf8() {
        let s = "Hello, world!";
        let debug = format!("{s:?}");

        let val1 = MaybeUtf8::from(s);

        assert!(val1.is_utf8());

        assert_eq!(val1, s);
        assert_eq!(val1.as_str(), Some(s));

        assert_eq!(val1, val1);

        assert_eq!(format!("{val1:?}"), debug);
        assert_eq!(val1.to_string(), s);

        let val2 = MaybeUtf8::from(s.to_string());

        assert!(val2.is_utf8());

        assert_eq!(val2, s);
        assert_eq!(val2.as_str(), Some(s));

        assert_eq!(val1, val2);
        assert_eq!(val2, val2);

        assert_eq!(format!("{val2:?}"), debug);
        assert_eq!(val2.to_string(), s);

        let val3 = MaybeUtf8::from_string(s);
        assert!(val3.is_utf8());

        assert_eq!(val3, s);
        assert_eq!(val3.as_str(), Some(s));

        assert_eq!(val1, val3);
        assert_eq!(val2, val3);
        assert_eq!(val3, val3);

        assert_eq!(format!("{val3:?}"), debug);
        assert_eq!(val3.to_string(), s);

        let Ok(val4) = s.parse::<MaybeUtf8>();

        assert!(val4.is_utf8());

        assert_eq!(val4, s);
        assert_eq!(val4.as_str(), Some(s));

        assert_eq!(val1, val4);
        assert_eq!(val2, val4);
        assert_eq!(val3, val4);
        assert_eq!(val4, val4);

        assert_eq!(format!("{val4:?}"), debug);
        assert_eq!(val4.to_string(), s);
    }

    #[test]
    fn is_not_utf8() {
        let bytes: &'static [u8] = &[0xFFu8; 16];
        let debug = format!("\"{}\"", bytes.escape_ascii());

        let val1 = MaybeUtf8::from(bytes);

        assert!(!val1.is_utf8());

        assert_eq!(val1, bytes);
        assert_eq!(val1.as_str(), None);
        assert_eq!(val1.as_bytes(), bytes);

        assert_eq!(val1, val1);

        assert_eq!(format!("{val1:?}"), debug);
        assert_eq!(val1.to_string(), bytes.escape_ascii().to_string());

        let val2 = MaybeUtf8::from(bytes.to_vec());

        assert!(!val2.is_utf8());

        assert_eq!(val2, bytes);
        assert_eq!(val2.as_str(), None);
        assert_eq!(val2.as_bytes(), bytes);

        assert_eq!(val1, val2);
        assert_eq!(val2, val2);

        assert_eq!(format!("{val2:?}"), debug);

        let val3 = MaybeUtf8::from_bytes(bytes);

        assert!(!val3.is_utf8());

        assert_eq!(val3, bytes);
        assert_eq!(val3.as_str(), None);
        assert_eq!(val3.as_bytes(), bytes);

        assert_eq!(val1, val3);
        assert_eq!(val2, val3);
        assert_eq!(val3, val3);

        assert_eq!(format!("{val3:?}"), debug);
    }

    #[test]
    fn equivalent_hashes_utf8() {
        let hasher = RandomState::new();

        let s = "Hello, world!";

        assert_eq!(hasher.hash_one(s), hasher.hash_one(MaybeUtf8::from(s)));
        assert_eq!(
            hasher.hash_one(s),
            hasher.hash_one(MaybeUtf8::from(s.to_string()))
        );
        assert_eq!(
            hasher.hash_one(s),
            hasher.hash_one(MaybeUtf8::from_string(s))
        );
        assert_eq!(
            hasher.hash_one(s),
            hasher.hash_one(s.parse::<MaybeUtf8>().unwrap())
        );
    }

    #[test]
    fn equivalent_hashes_not_utf8() {
        let hasher = RandomState::new();

        let bytes: &'static [u8] = &[0xC0; 32];

        assert_eq!(
            hasher.hash_one(bytes),
            hasher.hash_one(MaybeUtf8::from(bytes))
        );
        assert_eq!(
            hasher.hash_one(bytes),
            hasher.hash_one(MaybeUtf8::from(bytes.to_vec()))
        );
        assert_eq!(
            hasher.hash_one(bytes),
            hasher.hash_one(MaybeUtf8::from_bytes(bytes))
        );
    }
}
