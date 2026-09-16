//! Contains [`Error`] and corresponding [`Result`].

use crate::native::BlockReadError;
use serde::{de, ser};
use std::{error::Error as StdError, fmt, io, result, str::Utf8Error};

/// A result with a specified [`Error`] type.
pub type Result<T, E = Error> = result::Result<T, E>;

pub(crate) type BoxedError = Box<dyn StdError + Send + Sync>;

/// Represents all possible errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum Error {
    #[error("invalid params: {0}")]
    InvalidParams(#[source] BoxedError),
    #[error("network error: {0}")]
    Network(#[source] BoxedError),
    #[error("compression error: {0}")]
    Compression(#[source] BoxedError),
    #[error("decompression error: {0}")]
    Decompression(#[source] BoxedError),
    #[error("data format error: {0}")]
    DataFormat(#[source] BoxedError),
    #[error("no rows returned by a query that expected to return at least one row")]
    RowNotFound,
    #[error("sequences must have a known size ahead of time")]
    SequenceMustHaveLength,
    #[error("`deserialize_any` is not supported")]
    DeserializeAnyNotSupported,
    #[error("not enough data, probably a row type mismatches a database schema")]
    NotEnoughData,
    #[error("string is not valid utf8")]
    InvalidUtf8Encoding(#[from] Utf8Error),
    #[error("tag for enum is not valid")]
    InvalidTagEncoding(usize),
    #[error("max number of types in the Variant data type is 255, got {0}")]
    VariantDiscriminatorIsOutOfBound(usize),
    #[error("a custom error message from serde: {0}")]
    Custom(String),
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("timeout expired")]
    TimedOut,
    #[error("error while parsing columns header from the response: {0}")]
    InvalidColumnsHeader(#[source] BoxedError),
    #[error("schema mismatch: {0}")]
    SchemaMismatch(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(BoxedError),
}

impl From<clickhouse_types::error::TypesError> for Error {
    fn from(err: clickhouse_types::error::TypesError) -> Self {
        Self::InvalidColumnsHeader(Box::new(err))
    }
}

impl From<hyper::Error> for Error {
    fn from(error: hyper::Error) -> Self {
        Self::Network(Box::new(error))
    }
}

impl From<hyper_util::client::legacy::Error> for Error {
    fn from(error: hyper_util::client::legacy::Error) -> Self {
        #[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
        if error.is_connect() {
            static SCHEME_IS_NOT_HTTP: &str = "invalid URL, scheme is not http";

            let src = error.source().unwrap();
            // Unfortunately, this seems to be the only way, as `INVALID_NOT_HTTP` is not public.
            // See https://github.com/hyperium/hyper-util/blob/v0.1.14/src/client/legacy/connect/http.rs#L491-L495
            if src.to_string() == SCHEME_IS_NOT_HTTP {
                return Self::Unsupported(format!(
                    "{SCHEME_IS_NOT_HTTP}; if you are trying to connect via HTTPS, \
                    consider enabling `native-tls` or `rustls-tls` feature"
                ));
            }
        }
        Self::Network(Box::new(error))
    }
}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        io::Error::other(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        // TODO: after MSRV 1.79 replace with `io::Error::downcast`.
        if error.get_ref().is_some_and(|r| r.is::<Error>()) {
            *error.into_inner().unwrap().downcast::<Error>().unwrap()
        } else {
            Self::Other(error.into())
        }
    }
}

impl From<BlockReadError> for Error {
    fn from(error: BlockReadError) -> Self {
        Self::DataFormat(Box::new(error))
    }
}

impl Error {
    /// https://opentelemetry.io/docs/specs/semconv/registry/attributes/error/#error-type
    #[cfg(feature = "opentelemetry")]
    pub(crate) fn error_type(&self) -> &str {
        match self {
            Error::InvalidParams(_) => "InvalidParams",
            Error::Network(_) => "Network",
            Error::Compression(_) => "Compression",
            Error::Decompression(_) => "Decompression",
            Error::DataFormat(_) => "DataFormat",
            Error::RowNotFound => "RowNotFound",
            Error::SequenceMustHaveLength => "SequenceMustHaveLength",
            Error::DeserializeAnyNotSupported => "DeserializeAnyNotSupported",
            Error::NotEnoughData => "NotEnoughData",
            Error::InvalidUtf8Encoding(_) => "InvalidUtf8Encoding",
            Error::InvalidTagEncoding(_) => "InvalidTagEncoding",
            Error::VariantDiscriminatorIsOutOfBound(_) => "VariantDiscriminatorIsOutOfBound",
            Error::Custom(_) => "Custom",
            Error::BadResponse(_) => "BadResponse",
            Error::TimedOut => "TimedOut",
            Error::InvalidColumnsHeader(_) => "InvalidColumnsHeader",
            Error::SchemaMismatch(_) => "SchemaMismatch",
            Error::Unsupported(_) => "Unsupported",
            Error::Other(_) => "Other",
        }
    }

    /// Record this `Error` in the context of the current `tracing::Span`,
    /// setting the OpenTelemetry conventional fields if the `opentelemetry` feature is enabled.
    pub(crate) fn record_in_current_span(&self, msg: &str) {
        // Span fields that remain unpopulated are not reported,
        // so we can avoid adding noise to logs if the user isn't utilizing this feature.
        #[cfg(feature = "opentelemetry")]
        tracing::record_all!(
            tracing::Span::current(),
            otel.status_code = "Error",
            otel.status_description = format!("{msg}: {self}"),
            error.type = self.error_type(),
        );

        tracing::debug!(error=%self, "{msg}");
    }
}

#[cfg(tests)]
mod tests {
    use crate::error::Error;
    use std::io;

    #[test]
    fn roundtrip_io_error() {
        let orig = Error::NotEnoughData;

        // Error -> io::Error
        let orig_str = orig.to_string();
        let io = io::Error::from(orig);
        assert_eq!(io.kind(), io::ErrorKind::Other);
        assert_eq!(io.to_string(), orig_str);

        // io::Error -> Error
        let orig = Error::from(io);
        assert!(matches!(orig, Error::NotEnoughData));
    }

    #[test]
    fn error_traits() {
        fn assert_traits<T: std::error::Error + Send + Sync>() {}

        assert_traits::<Error>();
    }
}
