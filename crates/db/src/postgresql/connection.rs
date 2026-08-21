use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use one_core::storage::DbConnectionConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig as RustlsClientConfig, DigitallySignedStruct,
    Error as RustlsError, RootCertStore, SignatureScheme,
};
use tokio::sync::Mutex;
use tokio_postgres::{
    Client, Config, NoTls, Row, Statement,
    config::SslMode,
    types::{FromSql, Type},
};
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{debug, error, info, warn};

use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    BinaryCell, ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult,
    SqlSource,
};
use crate::rustls_provider::ensure_rustls_crypto_provider;
use crate::ssh_tunnel::resolve_connection_target;
use crate::{DatabasePlugin, format_message, truncate_str};
use connection_tunnel::TunnelGuard;
use tokio::sync::mpsc;

const NUMERIC_POSITIVE: u16 = 0x0000;
const NUMERIC_NEGATIVE: u16 = 0x4000;
const NUMERIC_NAN: u16 = 0xC000;
const NUMERIC_POSITIVE_INFINITY: u16 = 0xD000;
const NUMERIC_NEGATIVE_INFINITY: u16 = 0xF000;

/// Normalize character-length syntax emitted by some PostgreSQL-compatible
/// databases before sending a statement to standard PostgreSQL.
///
/// The compatibility form `varchar(20 char)` is accepted by some servers but
/// is invalid PostgreSQL syntax. This scanner deliberately leaves strings,
/// quoted identifiers, comments, and dollar-quoted bodies untouched because
/// those regions may contain arbitrary user data.
fn normalize_postgres_compatibility_sql(sql: &str) -> std::borrow::Cow<'_, str> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut normalized = None;
    let mut copied_until = 0;

    while index < bytes.len() {
        if bytes[index] == b'\'' {
            index = skip_postgres_single_quote(bytes, index);
            continue;
        }
        if bytes[index] == b'"' {
            index = skip_postgres_quoted_identifier(bytes, index);
            continue;
        }
        if bytes[index..].starts_with(b"--") {
            index = skip_postgres_line_comment(bytes, index);
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index = skip_postgres_block_comment(bytes, index);
            continue;
        }
        if bytes[index] == b'$' {
            if let Some(end) = skip_postgres_dollar_quote(bytes, index) {
                index = end;
                continue;
            }
        }
        if !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }

        let Some((char_start, close_paren)) = postgres_compatibility_char_suffix_range(sql, index)
        else {
            index += 1;
            continue;
        };

        if normalized.is_none() {
            normalized = Some(String::with_capacity(sql.len()));
        }
        let output = normalized.as_mut().expect("normalized output initialized");
        output.push_str(&sql[copied_until..char_start]);
        copied_until = close_paren;
        index = close_paren;
    }

    match normalized {
        Some(mut normalized) => {
            normalized.push_str(&sql[copied_until..]);
            std::borrow::Cow::Owned(normalized)
        }
        None => std::borrow::Cow::Borrowed(sql),
    }
}

fn postgres_compatibility_char_suffix_range(sql: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = sql.as_bytes();
    if !is_postgres_word_boundary(bytes, start.checked_sub(1)) {
        return None;
    }

    let first_word_end = bytes[start..]
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(*byte, b'_' | b'$'))
        .map(|offset| start + offset)
        .unwrap_or(bytes.len());
    let first_word = &sql[start..first_word_end];
    let mut type_end =
        if first_word.eq_ignore_ascii_case("varchar") || first_word.eq_ignore_ascii_case("char") {
            first_word_end
        } else if first_word.eq_ignore_ascii_case("character") {
            let varying_start = skip_postgres_ascii_whitespace(bytes, first_word_end);
            let varying_end = varying_start.saturating_add("varying".len());
            if varying_end <= bytes.len()
                && sql[varying_start..varying_end].eq_ignore_ascii_case("varying")
                && is_postgres_word_boundary(bytes, Some(varying_end))
            {
                varying_end
            } else {
                first_word_end
            }
        } else {
            return None;
        };

    if !is_postgres_word_boundary(bytes, Some(type_end)) {
        return None;
    }
    type_end = skip_postgres_ascii_whitespace(bytes, type_end);
    if bytes.get(type_end) != Some(&b'(') {
        return None;
    }

    let digit_start = skip_postgres_ascii_whitespace(bytes, type_end + 1);
    let mut digit_end = digit_start;
    while bytes
        .get(digit_end)
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        digit_end += 1;
    }
    if digit_end == digit_start {
        return None;
    }

    let char_start = skip_postgres_ascii_whitespace(bytes, digit_end);
    let char_end = char_start.saturating_add("char".len());
    if char_start == digit_end
        || char_end > bytes.len()
        || !sql[char_start..char_end].eq_ignore_ascii_case("char")
        || !is_postgres_word_boundary(bytes, Some(char_end))
    {
        return None;
    }

    let close_paren = skip_postgres_ascii_whitespace(bytes, char_end);
    if bytes.get(close_paren) == Some(&b')') {
        Some((digit_end, close_paren))
    } else {
        None
    }
}

fn skip_postgres_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn is_postgres_word_boundary(bytes: &[u8], index: Option<usize>) -> bool {
    index
        .and_then(|index| bytes.get(index))
        .is_none_or(|byte| !byte.is_ascii_alphanumeric() && !matches!(*byte, b'_' | b'$'))
}

fn skip_postgres_single_quote(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() => index += 2,
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_postgres_quoted_identifier(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if bytes.get(index + 1) == Some(&b'"') {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_postgres_line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(bytes.len())
}

fn skip_postgres_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    let mut depth = 1;
    while index < bytes.len() && depth > 0 {
        if bytes[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"*/") {
            depth -= 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    index
}

fn skip_postgres_dollar_quote(bytes: &[u8], start: usize) -> Option<usize> {
    let mut tag_end = start + 1;
    while tag_end < bytes.len() && bytes[tag_end] != b'$' {
        let byte = bytes[tag_end];
        if !byte.is_ascii_alphanumeric() && byte != b'_' {
            return None;
        }
        tag_end += 1;
    }
    if bytes.get(tag_end) != Some(&b'$')
        || (tag_end > start + 1
            && !bytes[start + 1].is_ascii_alphabetic()
            && bytes[start + 1] != b'_')
    {
        return None;
    }

    let tag = &bytes[start..=tag_end];
    bytes[tag_end + 1..]
        .windows(tag.len())
        .position(|window| window == tag)
        .map(|offset| tag_end + 1 + offset + tag.len())
        .or(Some(bytes.len()))
}

#[derive(Debug)]
struct PostgresNumeric(String);

impl<'a> FromSql<'a> for PostgresNumeric {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        decode_numeric(raw)
            .map(Self)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message).into())
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}

fn decode_numeric(raw: &[u8]) -> std::result::Result<String, String> {
    if raw.len() < 8 {
        return Err("PostgreSQL NUMERIC value is shorter than its header".to_string());
    }
    let ndigits = i16::from_be_bytes([raw[0], raw[1]]);
    let weight = i16::from_be_bytes([raw[2], raw[3]]);
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let scale = u16::from_be_bytes([raw[6], raw[7]]);
    if ndigits < 0 || raw.len() != 8 + ndigits as usize * 2 {
        return Err("PostgreSQL NUMERIC value has an invalid digit count".to_string());
    }
    let digits = raw[8..]
        .chunks_exact(2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    if digits.iter().any(|digit| *digit > 9999) {
        return Err("PostgreSQL NUMERIC value contains an invalid base-10000 digit".to_string());
    }
    match sign {
        NUMERIC_POSITIVE => Ok(format_finite_numeric(weight, scale, &digits, false)),
        NUMERIC_NEGATIVE => Ok(format_finite_numeric(weight, scale, &digits, true)),
        NUMERIC_NAN => Ok("NaN".to_string()),
        NUMERIC_POSITIVE_INFINITY => Ok("Infinity".to_string()),
        NUMERIC_NEGATIVE_INFINITY => Ok("-Infinity".to_string()),
        _ => Err(format!(
            "PostgreSQL NUMERIC value has unknown sign {sign:#06x}"
        )),
    }
}

fn format_finite_numeric(weight: i16, scale: u16, digits: &[u16], negative: bool) -> String {
    let mut value = String::new();
    if negative {
        value.push('-');
    }
    let integer_groups = (i32::from(weight) + 1).max(0) as usize;
    if integer_groups == 0 {
        value.push('0');
    } else {
        for group in 0..integer_groups {
            let digit = digits.get(group).copied().unwrap_or(0);
            if group == 0 {
                value.push_str(&digit.to_string());
            } else {
                value.push_str(&format!("{digit:04}"));
            }
        }
    }
    if scale > 0 {
        value.push('.');
        let fraction_groups = usize::from(scale).div_ceil(4);
        for group in 1..=fraction_groups {
            let index = i32::from(weight) + group as i32;
            let digit = usize::try_from(index)
                .ok()
                .and_then(|index| digits.get(index))
                .copied()
                .unwrap_or(0);
            value.push_str(&format!("{digit:04}"));
        }
        value.truncate(value.len() - (fraction_groups * 4 - usize::from(scale)));
    }
    value
}

#[derive(Debug)]
struct PostgresServerCertVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    accept_invalid_certs: bool,
    accept_invalid_hostnames: bool,
}

impl PostgresServerCertVerifier {
    fn new(
        inner: Arc<dyn ServerCertVerifier>,
        accept_invalid_certs: bool,
        accept_invalid_hostnames: bool,
    ) -> Self {
        Self {
            inner,
            accept_invalid_certs,
            accept_invalid_hostnames,
        }
    }

    fn should_ignore_certificate_error(&self, error: &CertificateError) -> bool {
        if self.accept_invalid_certs {
            return self.accept_invalid_hostnames
                || !matches!(
                    error,
                    CertificateError::NotValidForName
                        | CertificateError::NotValidForNameContext { .. }
                );
        }

        self.accept_invalid_hostnames
            && matches!(
                error,
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. }
            )
    }
}

impl ServerCertVerifier for PostgresServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => Ok(verified),
            Err(RustlsError::InvalidCertificate(error))
                if self.should_ignore_certificate_error(&error) =>
            {
                Ok(ServerCertVerified::assertion())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

pub struct PostgresDbConnection {
    config: DbConnectionConfig,
    client: Arc<Mutex<Option<Client>>>,
    tunnel: Option<TunnelGuard>,
}

impl PostgresDbConnection {
    fn is_loopback_host(host: &str) -> bool {
        matches!(
            host.trim().to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "::1"
        )
    }

    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(None)),
            tunnel: None,
        }
    }

    fn ssl_mode(config: &DbConnectionConfig) -> SslMode {
        match config
            .get_param("ssl_mode")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("disable") => SslMode::Disable,
            Some("require") => SslMode::Require,
            Some("prefer") => SslMode::Prefer,
            _ if Self::is_loopback_host(&config.host) => SslMode::Disable,
            _ => SslMode::Prefer,
        }
    }

    fn load_root_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, DbError> {
        let cert_bytes = fs::read(path).map_err(|error| {
            DbError::connection(format!(
                "failed to read PostgreSQL root certificate: {}",
                error
            ))
        })?;

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("der") => Ok(vec![CertificateDer::from(cert_bytes)]),
            _ => {
                let mut reader = BufReader::new(cert_bytes.as_slice());
                let certificates = rustls_pemfile::certs(&mut reader)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        DbError::connection_with_source("invalid PEM certificate", error)
                    })?;

                if certificates.is_empty() {
                    return Err(DbError::connection(
                        "PostgreSQL root certificate file does not contain any certificates",
                    ));
                }

                Ok(certificates)
            }
        }
    }

    fn build_root_cert_store(config: &DbConnectionConfig) -> Result<RootCertStore, DbError> {
        let build_started = Instant::now();
        let mut root_store = RootCertStore::empty();
        let native_load_started = Instant::now();
        let native_certificates = rustls_native_certs::load_native_certs();
        let native_cert_count = native_certificates.certs.len();
        let native_error_count = native_certificates.errors.len();
        info!(
            "[PostgreSQL][Timing] load_native_certs={}ms certs={} errors={}",
            native_load_started.elapsed().as_millis(),
            native_cert_count,
            native_error_count
        );

        for error in native_certificates.errors {
            warn!(
                "[PostgreSQL] Failed to load native root certificate: {}",
                error
            );
        }
        for certificate in native_certificates.certs {
            root_store.add(certificate).map_err(|error| {
                DbError::connection_with_source(
                    "failed to add native PostgreSQL root certificate",
                    error,
                )
            })?;
        }

        if let Some(path) = config
            .get_param("ssl_root_cert_path")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let custom_load_started = Instant::now();
            let certificates = Self::load_root_certificates(Path::new(path))?;
            info!(
                "[PostgreSQL][Timing] load_custom_root_certs={}ms path={} certs={}",
                custom_load_started.elapsed().as_millis(),
                path,
                certificates.len()
            );

            for certificate in certificates {
                root_store.add(certificate).map_err(|error| {
                    DbError::connection_with_source(
                        "failed to add PostgreSQL root certificate",
                        error,
                    )
                })?;
            }
        }

        info!(
            "[PostgreSQL][Timing] build_root_cert_store={}ms",
            build_started.elapsed().as_millis()
        );
        Ok(root_store)
    }

    fn build_tls_connector(config: &DbConnectionConfig) -> Result<MakeRustlsConnect, DbError> {
        let build_started = Instant::now();
        ensure_rustls_crypto_provider();

        let accept_invalid_certs = config.get_param_bool("ssl_accept_invalid_certs");
        let accept_invalid_hostnames = config.get_param_bool("ssl_accept_invalid_hostnames");
        let root_store = Self::build_root_cert_store(config)?;
        let base_verifier: Arc<dyn ServerCertVerifier> =
            rustls::client::WebPkiServerVerifier::builder(root_store.into())
                .build()
                .map_err(|error| {
                    DbError::connection(format!(
                        "failed to build PostgreSQL certificate verifier: {}",
                        error
                    ))
                })?;
        let verifier: Arc<dyn ServerCertVerifier> =
            if accept_invalid_certs || accept_invalid_hostnames {
                Arc::new(PostgresServerCertVerifier::new(
                    base_verifier,
                    accept_invalid_certs,
                    accept_invalid_hostnames,
                ))
            } else {
                base_verifier
            };

        let client_config = RustlsClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        info!(
            "[PostgreSQL][Timing] build_tls_connector={}ms invalid_certs={} invalid_hostnames={}",
            build_started.elapsed().as_millis(),
            accept_invalid_certs,
            accept_invalid_hostnames
        );
        Ok(MakeRustlsConnect::new(client_config))
    }

    /// Extract value from PostgreSQL row
    fn extract_value(row: &Row, index: usize) -> Option<String> {
        // Get column type
        let column = &row.columns()[index];
        let col_type = column.type_();

        // Try to get the value based on type
        match col_type {
            // Boolean
            &Type::BOOL => row
                .try_get::<_, Option<bool>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),

            // Integer types
            &Type::INT2 => row
                .try_get::<_, Option<i16>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),
            &Type::INT4 => row
                .try_get::<_, Option<i32>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),
            &Type::INT8 => row
                .try_get::<_, Option<i64>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),
            // PostgreSQL's internal single-byte `"char"` type is distinct from
            // SQL CHAR(n), which is reported as BPCHAR.
            &Type::CHAR => row
                .try_get::<_, Option<i8>>(index)
                .ok()
                .flatten()
                .map(|v| char::from(v as u8).to_string()),

            // Floating point types
            &Type::FLOAT4 => row
                .try_get::<_, Option<f32>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),
            &Type::FLOAT8 => row
                .try_get::<_, Option<f64>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),

            // NUMERIC is arbitrary precision and must not be coerced through f64.
            &Type::NUMERIC => row
                .try_get::<_, Option<PostgresNumeric>>(index)
                .ok()
                .flatten()
                .map(|v| v.0),

            // Text types
            &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR | &Type::NAME => {
                row.try_get::<_, Option<String>>(index).ok().flatten()
            }

            // Date and Time types
            &Type::TIMESTAMP => {
                use chrono::NaiveDateTime;
                row.try_get::<_, Option<NaiveDateTime>>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
            }
            &Type::TIMESTAMPTZ => {
                use chrono::{DateTime, Utc};
                row.try_get::<_, Option<DateTime<Utc>>>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.format("%Y-%m-%d %H:%M:%S %z").to_string())
            }
            &Type::DATE => {
                use chrono::NaiveDate;
                row.try_get::<_, Option<NaiveDate>>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.format("%Y-%m-%d").to_string())
            }
            &Type::TIME => {
                use chrono::NaiveTime;
                row.try_get::<_, Option<NaiveTime>>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.format("%H:%M:%S").to_string())
            }

            // Binary types
            &Type::BYTEA => row
                .try_get::<_, Option<Vec<u8>>>(index)
                .ok()
                .flatten()
                .map(|v| format!("\\x{}", hex::encode(&v))),

            // JSON types
            &Type::JSON | &Type::JSONB => row
                .try_get::<_, Option<serde_json::Value>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),

            // UUID
            &Type::UUID => row
                .try_get::<_, Option<uuid::Uuid>>(index)
                .ok()
                .flatten()
                .map(|v| v.to_string()),

            // Array types - try to get as string representation
            _ if col_type.name().ends_with("[]") => {
                // For arrays, try to get as string
                row.try_get::<_, Option<String>>(index)
                    .ok()
                    .flatten()
                    .or_else(|| Some(format!("<array: {}>", col_type.name())))
            }

            // Default: try as string, otherwise show type info
            _ => row
                .try_get::<_, Option<String>>(index)
                .ok()
                .flatten()
                .or_else(|| Some(format!("<{}>", col_type.name()))),
        }
    }

    fn build_query_result(
        stmt: &Statement,
        rows: Vec<Row>,
        sql: String,
        elapsed_ms: u128,
    ) -> SqlResult {
        let columns: Vec<String> = stmt
            .columns()
            .iter()
            .map(|col| col.name().to_string())
            .collect();

        let column_meta: Vec<QueryColumnMeta> = stmt
            .columns()
            .iter()
            .map(|col| {
                let name = col.name().to_string();
                let db_type = col.type_().name().to_string();
                QueryColumnMeta::new(name, db_type)
            })
            .collect();

        let mut binary_cells = Vec::new();
        let all_rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                (0..columns.len())
                    .map(|i| {
                        if stmt.columns()[i].type_() == &Type::BYTEA {
                            if let Some(bytes) = row.try_get::<_, Option<Vec<u8>>>(i).ok().flatten()
                            {
                                binary_cells.push(BinaryCell {
                                    row_index,
                                    column_index: i,
                                    bytes,
                                });
                            }
                        }
                        Self::extract_value(row, i)
                    })
                    .collect()
            })
            .collect();

        SqlResult::Query(QueryResult {
            sql,
            columns,
            column_meta,
            rows: all_rows,
            binary_cells,
            elapsed_ms,
        })
    }

    fn build_exec_result(sql: String, rows_affected: u64, elapsed_ms: u128) -> SqlResult {
        let message = format_message(&sql, rows_affected);
        SqlResult::Exec(ExecResult {
            sql,
            rows_affected,
            elapsed_ms,
            message: Some(message),
        })
    }

    fn should_retry_without_tls(error: &dyn std::error::Error) -> bool {
        let mut current = Some(error);
        while let Some(err) = current {
            let message = err.to_string().to_ascii_lowercase();
            if message.contains("invalid peer certificate")
                || message.contains("unsupportedcertversion")
                || message.contains("unsupported cert version")
                || message.contains("certificate verify failed")
            {
                return true;
            }
            current = err.source();
        }
        false
    }

    fn format_server_error_message(
        message: &str,
        code: &str,
        detail: Option<&str>,
        hint: Option<&str>,
        where_: Option<&str>,
        schema: Option<&str>,
        table: Option<&str>,
        column: Option<&str>,
        datatype: Option<&str>,
        constraint: Option<&str>,
    ) -> String {
        let mut formatted = format!("{message} (SQLSTATE {code})");

        if let Some(detail) = detail.filter(|value| !value.is_empty()) {
            formatted.push_str(&format!("\nDetail: {detail}"));
        }
        if let Some(hint) = hint.filter(|value| !value.is_empty()) {
            formatted.push_str(&format!("\nHint: {hint}"));
        }
        if let Some(where_) = where_.filter(|value| !value.is_empty()) {
            formatted.push_str(&format!("\nWhere: {where_}"));
        }

        let context = [
            ("schema", schema),
            ("table", table),
            ("column", column),
            ("datatype", datatype),
            ("constraint", constraint),
        ]
        .into_iter()
        .filter_map(|(label, value)| {
            value
                .filter(|value| !value.is_empty())
                .map(|value| format!("{label}: {value}"))
        })
        .collect::<Vec<_>>();
        if !context.is_empty() {
            formatted.push_str(&format!("\nContext: {}", context.join(", ")));
        }

        formatted
    }

    fn postgres_error_message(error: &tokio_postgres::Error) -> String {
        let Some(db_error) = error.as_db_error() else {
            return error.to_string();
        };

        Self::format_server_error_message(
            db_error.message(),
            db_error.code().code(),
            db_error.detail(),
            db_error.hint(),
            db_error.where_(),
            db_error.schema(),
            db_error.table(),
            db_error.column(),
            db_error.datatype(),
            db_error.constraint(),
        )
    }

    fn postgres_connection_error(context: &str, error: tokio_postgres::Error) -> DbError {
        let message = format!("{context}: {}", Self::postgres_error_message(&error));
        DbError::Connection {
            message,
            source: Some(Box::new(error)),
        }
    }

    fn postgres_query_error(context: &str, error: tokio_postgres::Error) -> DbError {
        let message = format!("{context}: {}", Self::postgres_error_message(&error));
        DbError::Query {
            message,
            source: Some(Box::new(error)),
        }
    }

    fn postgres_transaction_error(context: &str, error: tokio_postgres::Error) -> DbError {
        let message = format!("{context}: {}", Self::postgres_error_message(&error));
        DbError::Transaction {
            message,
            source: Some(Box::new(error)),
        }
    }

    fn connection_error(error: tokio_postgres::Error) -> DbError {
        Self::postgres_connection_error("failed to connect", error)
    }

    async fn connect_without_tls(pg_config: &Config) -> Result<Client, tokio_postgres::Error> {
        let (client, connection) = pg_config.connect(NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                error!(
                    "[PostgreSQL] Connection error: {}",
                    Self::postgres_error_message(&error)
                );
            }
        });
        Ok(client)
    }

    async fn connect_with_tls(
        pg_config: &Config,
        tls_connector: MakeRustlsConnect,
    ) -> Result<Client, tokio_postgres::Error> {
        let (client, connection) = pg_config.connect(tls_connector).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                error!(
                    "[PostgreSQL] Connection error: {}",
                    Self::postgres_error_message(&error)
                );
            }
        });
        Ok(client)
    }
}

#[async_trait]
impl DbConnection for PostgresDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    fn supports_database_switch(&self) -> bool {
        false
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let connect_started = Instant::now();
        let config = &self.config;
        info!("[PostgreSQL] Connecting to {}:{}", config.host, config.port);
        let resolve_started = Instant::now();
        let target = resolve_connection_target(config).await?;
        let resolve_elapsed_ms = resolve_started.elapsed().as_millis();
        self.tunnel = target.tunnel;
        info!(
            "[PostgreSQL][Timing] resolve_connection_target={}ms host={}:{} target={}:{} ssh_tunnel={}",
            resolve_elapsed_ms,
            config.host,
            config.port,
            target.host,
            target.port,
            self.tunnel.is_some()
        );

        let mut pg_config = Config::new();
        pg_config
            .host(&target.host)
            .port(target.port)
            .user(&config.username)
            .password(&config.password);

        if let Some(ref db) = config.database {
            pg_config.dbname(db);
            debug!("[PostgreSQL] Using database: {}", db);
        }

        // Apply extra params
        if let Some(timeout) = config.get_param_as::<u64>("connect_timeout") {
            pg_config.connect_timeout(std::time::Duration::from_secs(timeout));
            debug!("[PostgreSQL] Connect timeout: {}s", timeout);
        }
        if let Some(app_name) = config.get_param("application_name") {
            pg_config.application_name(app_name);
            debug!("[PostgreSQL] Application name: {}", app_name);
        }

        let ssl_mode = Self::ssl_mode(config);
        pg_config.ssl_mode(ssl_mode);

        // Connect to PostgreSQL
        debug!("[PostgreSQL] Establishing connection...");
        let client = match ssl_mode {
            SslMode::Disable => {
                let connect_without_tls_started = Instant::now();
                let client = Self::connect_without_tls(&pg_config)
                    .await
                    .map_err(|error| {
                        error!(
                            "[PostgreSQL] Connection failed: {}",
                            Self::postgres_error_message(&error)
                        );
                        Self::connection_error(error)
                    })?;
                info!(
                    "[PostgreSQL][Timing] connect_without_tls={}ms",
                    connect_without_tls_started.elapsed().as_millis()
                );
                client
            }
            SslMode::Prefer => {
                let tls_connector_started = Instant::now();
                let tls_connector = Self::build_tls_connector(config)?;
                info!(
                    "[PostgreSQL][Timing] prepare_tls_connector={}ms",
                    tls_connector_started.elapsed().as_millis()
                );
                let tls_connect_started = Instant::now();
                match Self::connect_with_tls(&pg_config, tls_connector).await {
                    Ok(client) => {
                        info!(
                            "[PostgreSQL][Timing] connect_with_tls={}ms ssl_mode=Prefer",
                            tls_connect_started.elapsed().as_millis()
                        );
                        client
                    }
                    Err(error) if Self::should_retry_without_tls(&error) => {
                        let error_message = Self::postgres_error_message(&error);
                        warn!(
                            "[PostgreSQL] TLS connect failed in prefer mode, retrying without TLS: {}",
                            error_message
                        );
                        info!(
                            "[PostgreSQL][Timing] connect_with_tls_failed={}ms ssl_mode=Prefer reason={}",
                            tls_connect_started.elapsed().as_millis(),
                            error_message
                        );
                        let retry_started = Instant::now();
                        let client = Self::connect_without_tls(&pg_config).await.map_err(
                            |retry_error| {
                                error!(
                                    "[PostgreSQL] Non-TLS retry after TLS failure also failed: {}",
                                    Self::postgres_error_message(&retry_error)
                                );
                                Self::connection_error(retry_error)
                            },
                        )?;
                        info!(
                            "[PostgreSQL][Timing] retry_without_tls={}ms ssl_mode=Prefer",
                            retry_started.elapsed().as_millis()
                        );
                        client
                    }
                    Err(error) => {
                        error!(
                            "[PostgreSQL] Connection failed: {}",
                            Self::postgres_error_message(&error)
                        );
                        return Err(Self::connection_error(error));
                    }
                }
            }
            _ => {
                let tls_connector_started = Instant::now();
                let tls_connector = Self::build_tls_connector(config)?;
                info!(
                    "[PostgreSQL][Timing] prepare_tls_connector={}ms ssl_mode={:?}",
                    tls_connector_started.elapsed().as_millis(),
                    ssl_mode
                );
                let tls_connect_started = Instant::now();
                let client = Self::connect_with_tls(&pg_config, tls_connector)
                    .await
                    .map_err(|error| {
                        error!(
                            "[PostgreSQL] Connection failed: {}",
                            Self::postgres_error_message(&error)
                        );
                        Self::connection_error(error)
                    })?;
                info!(
                    "[PostgreSQL][Timing] connect_with_tls={}ms ssl_mode={:?}",
                    tls_connect_started.elapsed().as_millis(),
                    ssl_mode
                );
                client
            }
        };

        {
            let mut guard = self.client.lock().await;
            *guard = Some(client);
        }

        info!(
            "[PostgreSQL] Connected successfully in {}ms (ssl_mode={:?})",
            connect_started.elapsed().as_millis(),
            ssl_mode
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        debug!("[PostgreSQL] Disconnecting...");
        let mut guard = self.client.lock().await;
        *guard = None;
        self.tunnel = None;
        info!("[PostgreSQL] Disconnected");
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        debug!(
            "[PostgreSQL] execute() called, transactional={}, stop_on_error={}",
            options.transactional, options.stop_on_error
        );
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let parser = plugin
            .create_parser(SqlSource::Script(script.to_string()))
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;
        let statements: Vec<String> = parser
            .filter_map(|r| r.ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        debug!("[PostgreSQL] Split into {} statement(s)", statements.len());
        let mut results = Vec::new();

        if options.transactional {
            debug!("[PostgreSQL] Starting transaction...");
            let tx = client.transaction().await.map_err(|e| {
                error!(
                    "[PostgreSQL] Failed to begin transaction: {}",
                    Self::postgres_error_message(&e)
                );
                Self::postgres_transaction_error("failed to begin transaction", e)
            })?;

            for (idx, sql) in statements.iter().enumerate() {
                let sql = sql.trim();
                if sql.is_empty() {
                    continue;
                }

                let sql_to_execute = plugin.apply_query_max_rows(sql, options.max_rows);
                let sql_to_execute = normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                let sql_to_execute = sql_to_execute.as_ref();
                let sql_preview = if sql_to_execute.len() > 200 {
                    format!("{}...", truncate_str(sql_to_execute, 200))
                } else {
                    sql_to_execute.to_string()
                };
                debug!(
                    "[PostgreSQL] TX executing statement {}/{}, {}",
                    idx + 1,
                    statements.len(),
                    sql_preview
                );
                let start = Instant::now();

                let result = match tx.prepare(sql_to_execute).await {
                    Ok(stmt) => {
                        if stmt.columns().is_empty() {
                            match tx.execute(&stmt, &[]).await {
                                Ok(rows_affected) => {
                                    let elapsed_ms = start.elapsed().as_millis();
                                    debug!(
                                        "[PostgreSQL] TX execute completed: {} rows affected, {}ms",
                                        rows_affected, elapsed_ms
                                    );
                                    Self::build_exec_result(
                                        sql_to_execute.to_string(),
                                        rows_affected,
                                        elapsed_ms,
                                    )
                                }
                                Err(e) => {
                                    error!(
                                        "[PostgreSQL] TX execute failed: {}, SQL: {}",
                                        Self::postgres_error_message(&e),
                                        sql_preview
                                    );
                                    SqlResult::Error(SqlErrorInfo {
                                        sql: sql_to_execute.to_string(),
                                        message: Self::postgres_error_message(&e),
                                    })
                                }
                            }
                        } else {
                            match tx.query(&stmt, &[]).await {
                                Ok(rows) => {
                                    let elapsed_ms = start.elapsed().as_millis();
                                    debug!(
                                        "[PostgreSQL] TX query completed: {} rows, {}ms",
                                        rows.len(),
                                        elapsed_ms
                                    );
                                    Self::build_query_result(
                                        &stmt,
                                        rows,
                                        sql_to_execute.to_string(),
                                        elapsed_ms,
                                    )
                                }
                                Err(e) => {
                                    error!(
                                        "[PostgreSQL] TX query failed: {}, SQL: {}",
                                        Self::postgres_error_message(&e),
                                        sql_preview
                                    );
                                    SqlResult::Error(SqlErrorInfo {
                                        sql: sql_to_execute.to_string(),
                                        message: Self::postgres_error_message(&e),
                                    })
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "[PostgreSQL] TX prepare failed: {}, SQL: {}",
                            Self::postgres_error_message(&e),
                            sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql_to_execute.to_string(),
                            message: Self::postgres_error_message(&e),
                        })
                    }
                };

                let is_error = result.is_error();
                results.push(result.with_original_sql(sql));

                if is_error {
                    debug!(
                        "[PostgreSQL] TX statement {}/{} returned error, will rollback",
                        idx + 1,
                        statements.len()
                    );
                    break;
                }
            }

            let has_error = results.iter().any(|r| r.is_error());
            if has_error {
                debug!("[PostgreSQL] Rolling back transaction...");
                tx.rollback().await.map_err(|e| {
                    error!(
                        "[PostgreSQL] Failed to rollback: {}",
                        Self::postgres_error_message(&e)
                    );
                    Self::postgres_transaction_error("failed to rollback", e)
                })?;
                debug!("[PostgreSQL] Transaction rolled back");
            } else {
                debug!("[PostgreSQL] Committing transaction...");
                tx.commit().await.map_err(|e| {
                    error!(
                        "[PostgreSQL] Failed to commit: {}",
                        Self::postgres_error_message(&e)
                    );
                    Self::postgres_transaction_error("failed to commit", e)
                })?;
                debug!("[PostgreSQL] Transaction committed");
            }
        } else {
            for (idx, sql) in statements.iter().enumerate() {
                let sql = sql.trim();
                if sql.is_empty() {
                    continue;
                }

                let sql_preview = if sql.len() > 200 {
                    format!("{}...", truncate_str(&sql, 200))
                } else {
                    sql.to_string()
                };

                // PostgreSQL doesn't have USE statement, but has SET search_path
                let sql_upper = sql.to_uppercase();
                if sql_upper.starts_with("SET SEARCH_PATH") {
                    debug!("[PostgreSQL] Executing search_path: {}", sql);
                    let start = Instant::now();
                    match client.execute(sql, &[]).await {
                        Ok(_) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!("[PostgreSQL] Search path changed, {}ms", elapsed_ms);
                            results.push(SqlResult::Exec(ExecResult {
                                sql: sql.to_string(),
                                rows_affected: 0,
                                elapsed_ms,
                                message: Some("Search path changed".to_string()),
                            }));
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Failed to change search path: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            results.push(SqlResult::Error(SqlErrorInfo {
                                sql: sql.to_string(),
                                message: Self::postgres_error_message(&e),
                            }));

                            if options.stop_on_error {
                                debug!(
                                    "[PostgreSQL] Stopping execution due to error (stop_on_error=true)"
                                );
                                break;
                            }
                        }
                    }
                    continue;
                }

                debug!(
                    "[PostgreSQL] Executing statement {}/{}",
                    idx + 1,
                    statements.len()
                );
                let start = Instant::now();
                let sql_to_execute = plugin.apply_query_max_rows(sql, options.max_rows);
                let sql_to_execute = normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                let sql_to_execute = sql_to_execute.as_ref();
                let sql_preview = if sql_to_execute.len() > 200 {
                    format!("{}...", truncate_str(sql_to_execute, 200))
                } else {
                    sql_to_execute.to_string()
                };

                let result = match client.prepare(sql_to_execute).await {
                    Ok(stmt) => {
                        if stmt.columns().is_empty() {
                            match client.execute(&stmt, &[]).await {
                                Ok(rows_affected) => {
                                    let elapsed_ms = start.elapsed().as_millis();
                                    debug!(
                                        "[PostgreSQL] Execute completed: {} rows affected, {}ms",
                                        rows_affected, elapsed_ms
                                    );
                                    Self::build_exec_result(
                                        sql_to_execute.to_string(),
                                        rows_affected,
                                        elapsed_ms,
                                    )
                                }
                                Err(e) => {
                                    error!(
                                        "[PostgreSQL] Execute failed: {}, SQL: {}",
                                        Self::postgres_error_message(&e),
                                        sql_preview
                                    );
                                    results.push(
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                        .with_original_sql(sql),
                                    );

                                    if options.stop_on_error {
                                        debug!(
                                            "[PostgreSQL] Stopping execution due to error (stop_on_error=true)"
                                        );
                                        break;
                                    }
                                    continue;
                                }
                            }
                        } else {
                            match client.query(&stmt, &[]).await {
                                Ok(rows) => {
                                    let elapsed_ms = start.elapsed().as_millis();
                                    debug!(
                                        "[PostgreSQL] Query completed: {} rows, {}ms",
                                        rows.len(),
                                        elapsed_ms
                                    );
                                    Self::build_query_result(
                                        &stmt,
                                        rows,
                                        sql_to_execute.to_string(),
                                        elapsed_ms,
                                    )
                                }
                                Err(e) => {
                                    error!(
                                        "[PostgreSQL] Query failed: {}, SQL: {}",
                                        Self::postgres_error_message(&e),
                                        sql_preview
                                    );
                                    results.push(
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                        .with_original_sql(sql),
                                    );

                                    if options.stop_on_error {
                                        debug!(
                                            "[PostgreSQL] Stopping execution due to error (stop_on_error=true)"
                                        );
                                        break;
                                    }
                                    continue;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "[PostgreSQL] Prepare failed: {}, SQL: {}",
                            Self::postgres_error_message(&e),
                            sql_preview
                        );
                        results.push(
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql_to_execute.to_string(),
                                message: Self::postgres_error_message(&e),
                            })
                            .with_original_sql(sql),
                        );

                        if options.stop_on_error {
                            debug!(
                                "[PostgreSQL] Stopping execution due to error (stop_on_error=true)"
                            );
                            break;
                        }
                        continue;
                    }
                };

                results.push(result.with_original_sql(sql));
            }
        }

        debug!(
            "[PostgreSQL] execute() completed with {} result(s)",
            results.len()
        );
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        debug!("[PostgreSQL] query() called");
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let start = Instant::now();
        let query_string = query.to_string();
        let sql_preview = if query.len() > 200 {
            format!("{}...", truncate_str(query, 200))
        } else {
            query.to_string()
        };
        debug!("[PostgreSQL] Executing query: {}", sql_preview);

        match client.prepare(&query_string).await {
            Ok(stmt) => {
                if stmt.columns().is_empty() {
                    match client.execute(&stmt, &vec![]).await {
                        Ok(rows_affected) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[PostgreSQL] Execute completed: {} rows affected, {}ms",
                                rows_affected, elapsed_ms
                            );
                            Ok(Self::build_exec_result(
                                query_string,
                                rows_affected,
                                elapsed_ms,
                            ))
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Execute failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            Ok(SqlResult::Error(SqlErrorInfo {
                                sql: query_string,
                                message: Self::postgres_error_message(&e),
                            }))
                        }
                    }
                } else {
                    match client.query(&stmt, &vec![]).await {
                        Ok(rows) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[PostgreSQL] Query completed: {} rows, {}ms",
                                rows.len(),
                                elapsed_ms
                            );
                            Ok(Self::build_query_result(
                                &stmt,
                                rows,
                                query_string,
                                elapsed_ms,
                            ))
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Query failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            Ok(SqlResult::Error(SqlErrorInfo {
                                sql: query_string,
                                message: Self::postgres_error_message(&e),
                            }))
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "[PostgreSQL] Prepare failed: {}, SQL: {}",
                    Self::postgres_error_message(&e),
                    sql_preview
                );
                Ok(SqlResult::Error(SqlErrorInfo {
                    sql: query_string,
                    message: Self::postgres_error_message(&e),
                }))
            }
        }
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        debug!("[PostgreSQL] Querying current database");
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let row = client
            .query_one("SELECT current_database()", &[])
            .await
            .map_err(|e| {
                error!(
                    "[PostgreSQL] Failed to get current database: {}",
                    Self::postgres_error_message(&e)
                );
                Self::postgres_query_error("failed to get current database", e)
            })?;

        let db = row.try_get::<_, Option<String>>(0).ok().flatten();
        debug!("[PostgreSQL] Current database: {:?}", db);
        Ok(db)
    }

    async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
        // PostgreSQL doesn't support switching databases within a connection
        // The connection must be recreated to connect to a different database
        error!("[PostgreSQL] Attempted to switch database - not supported");
        Err(DbError::NotSupported(
            "PostgreSQL does not support switching databases within a connection. Please create a new connection.".to_string()
        ))
    }

    async fn switch_schema(&self, schema: &str) -> Result<(), DbError> {
        debug!("[PostgreSQL] Switching to schema: {}", schema);
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let sql = format!("SET search_path TO \"{}\"", schema.replace("\"", "\"\""));
        debug!("[PostgreSQL] Executing: {}", sql);
        client.execute(&sql, &[]).await.map_err(|e| {
            error!(
                "[PostgreSQL] Failed to switch schema: {}, SQL: {}",
                Self::postgres_error_message(&e),
                sql
            );
            Self::postgres_query_error("failed to switch schema", e)
        })?;

        info!("[PostgreSQL] Switched to schema: {}", schema);
        Ok(())
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        debug!(
            "[PostgreSQL] execute_streaming() called, transactional={}, streaming={}",
            options.transactional, options.streaming
        );
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();

        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;
            let mut has_error = false;

            if options.transactional {
                debug!("[PostgreSQL] Starting transaction for streaming...");
                let tx = client.transaction().await.map_err(|e| {
                    error!(
                        "[PostgreSQL] Failed to begin transaction: {}",
                        Self::postgres_error_message(&e)
                    );
                    Self::postgres_transaction_error("failed to begin transaction", e)
                })?;

                while let Some(stmt_result) = parser.next() {
                    let bytes_read = parser.bytes_read();
                    let sql = match stmt_result {
                        Ok(s) if !s.trim().is_empty() => s,
                        Ok(_) => continue,
                        Err(e) => {
                            let progress = StreamingProgress::with_file_progress(
                                current,
                                SqlResult::Error(SqlErrorInfo {
                                    sql: String::new(),
                                    message: format!("Parse error: {}", e),
                                }),
                                bytes_read,
                                total_size,
                            );
                            let _ = sender.send(progress).await;
                            has_error = true;
                            break;
                        }
                    };

                    current += 1;
                    let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let sql_to_execute =
                        normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                    let sql_to_execute = sql_to_execute.as_ref();
                    let sql_preview = if sql_to_execute.len() > 200 {
                        format!("{}...", truncate_str(sql_to_execute, 200))
                    } else {
                        sql_to_execute.to_string()
                    };
                    debug!("[PostgreSQL] Streaming TX statement {}", current);
                    let start = Instant::now();

                    let result = match tx.prepare(sql_to_execute).await {
                        Ok(stmt) => {
                            if stmt.columns().is_empty() {
                                match tx.execute(&stmt, &[]).await {
                                    Ok(rows_affected) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_exec_result(
                                            sql_to_execute.to_string(),
                                            rows_affected,
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming TX execute failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            } else {
                                match tx.query(&stmt, &[]).await {
                                    Ok(rows) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_query_result(
                                            &stmt,
                                            rows,
                                            sql_to_execute.to_string(),
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming TX query failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Streaming TX prepare failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql_to_execute.to_string(),
                                message: Self::postgres_error_message(&e),
                            })
                        }
                    };

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    if is_error {
                        has_error = true;
                    }

                    let progress = StreamingProgress::with_file_progress(
                        current, result, bytes_read, total_size,
                    );
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error {
                        break;
                    }
                }

                if has_error {
                    tx.rollback().await.map_err(|e| {
                        error!(
                            "[PostgreSQL] Failed to rollback streaming transaction: {}",
                            Self::postgres_error_message(&e)
                        );
                        Self::postgres_transaction_error(
                            "failed to rollback streaming transaction",
                            e,
                        )
                    })?;
                } else {
                    tx.commit().await.map_err(|e| {
                        error!(
                            "[PostgreSQL] Failed to commit streaming transaction: {}",
                            Self::postgres_error_message(&e)
                        );
                        Self::postgres_transaction_error(
                            "failed to commit streaming transaction",
                            e,
                        )
                    })?;
                }
            } else {
                while let Some(stmt_result) = parser.next() {
                    let bytes_read = parser.bytes_read();
                    let sql = match stmt_result {
                        Ok(s) if !s.trim().is_empty() => s,
                        Ok(_) => continue,
                        Err(e) => {
                            let progress = StreamingProgress::with_file_progress(
                                current,
                                SqlResult::Error(SqlErrorInfo {
                                    sql: String::new(),
                                    message: format!("Parse error: {}", e),
                                }),
                                bytes_read,
                                total_size,
                            );
                            let _ = sender.send(progress).await;
                            if options.stop_on_error {
                                break;
                            }
                            continue;
                        }
                    };

                    current += 1;
                    let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let sql_to_execute =
                        normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                    let sql_to_execute = sql_to_execute.as_ref();
                    let sql_preview = if sql_to_execute.len() > 200 {
                        format!("{}...", truncate_str(sql_to_execute, 200))
                    } else {
                        sql_to_execute.to_string()
                    };
                    debug!("[PostgreSQL] Streaming statement {}", current);
                    let start = Instant::now();

                    let result = match client.prepare(sql_to_execute).await {
                        Ok(stmt) => {
                            if stmt.columns().is_empty() {
                                match client.execute(&stmt, &[]).await {
                                    Ok(rows_affected) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_exec_result(
                                            sql_to_execute.to_string(),
                                            rows_affected,
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming execute failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            } else {
                                match client.query(&stmt, &[]).await {
                                    Ok(rows) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_query_result(
                                            &stmt,
                                            rows,
                                            sql_to_execute.to_string(),
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming query failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Streaming prepare failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql_to_execute.to_string(),
                                message: Self::postgres_error_message(&e),
                            })
                        }
                    };

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    let progress = StreamingProgress::with_file_progress(
                        current, result, bytes_read, total_size,
                    );
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error && options.stop_on_error {
                        break;
                    }
                }
            }
        } else {
            let statements: Vec<String> = parser
                .filter_map(|r| r.ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let total = statements.len();
            debug!("[PostgreSQL] Streaming {} statement(s)", total);

            if options.transactional {
                debug!("[PostgreSQL] Starting transaction for streaming...");
                let tx = client.transaction().await.map_err(|e| {
                    error!(
                        "[PostgreSQL] Failed to begin transaction: {}",
                        Self::postgres_error_message(&e)
                    );
                    Self::postgres_transaction_error("failed to begin transaction", e)
                })?;

                let mut has_error = false;

                for (index, sql) in statements.into_iter().enumerate() {
                    let current = index + 1;
                    let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let sql_to_execute =
                        normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                    let sql_to_execute = sql_to_execute.as_ref();
                    let sql_preview = if sql_to_execute.len() > 200 {
                        format!("{}...", truncate_str(sql_to_execute, 200))
                    } else {
                        sql_to_execute.to_string()
                    };
                    debug!("[PostgreSQL] Streaming TX statement {}/{}", current, total);
                    let start = Instant::now();

                    let result = match tx.prepare(sql_to_execute).await {
                        Ok(stmt) => {
                            if stmt.columns().is_empty() {
                                match tx.execute(&stmt, &[]).await {
                                    Ok(rows_affected) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_exec_result(
                                            sql_to_execute.to_string(),
                                            rows_affected,
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming TX execute failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            } else {
                                match tx.query(&stmt, &[]).await {
                                    Ok(rows) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_query_result(
                                            &stmt,
                                            rows,
                                            sql_to_execute.to_string(),
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming TX query failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Streaming TX prepare failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql_to_execute.to_string(),
                                message: Self::postgres_error_message(&e),
                            })
                        }
                    };

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    if is_error {
                        has_error = true;
                    }

                    let progress = StreamingProgress::new(current, total, result);
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error {
                        break;
                    }
                }

                if has_error {
                    tx.rollback().await.map_err(|e| {
                        error!(
                            "[PostgreSQL] Failed to rollback streaming transaction: {}",
                            Self::postgres_error_message(&e)
                        );
                        Self::postgres_transaction_error(
                            "failed to rollback streaming transaction",
                            e,
                        )
                    })?;
                } else {
                    tx.commit().await.map_err(|e| {
                        error!(
                            "[PostgreSQL] Failed to commit streaming transaction: {}",
                            Self::postgres_error_message(&e)
                        );
                        Self::postgres_transaction_error(
                            "failed to commit streaming transaction",
                            e,
                        )
                    })?;
                }
            } else {
                for (index, sql) in statements.into_iter().enumerate() {
                    let current = index + 1;
                    let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let sql_to_execute =
                        normalize_postgres_compatibility_sql(sql_to_execute.as_ref());
                    let sql_to_execute = sql_to_execute.as_ref();
                    let sql_preview = if sql_to_execute.len() > 200 {
                        format!("{}...", truncate_str(sql_to_execute, 200))
                    } else {
                        sql_to_execute.to_string()
                    };
                    debug!("[PostgreSQL] Streaming statement {}/{}", current, total);
                    let start = Instant::now();

                    let result = match client.prepare(sql_to_execute).await {
                        Ok(stmt) => {
                            if stmt.columns().is_empty() {
                                match client.execute(&stmt, &[]).await {
                                    Ok(rows_affected) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_exec_result(
                                            sql_to_execute.to_string(),
                                            rows_affected,
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming execute failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            } else {
                                match client.query(&stmt, &[]).await {
                                    Ok(rows) => {
                                        let elapsed_ms = start.elapsed().as_millis();
                                        Self::build_query_result(
                                            &stmt,
                                            rows,
                                            sql_to_execute.to_string(),
                                            elapsed_ms,
                                        )
                                    }
                                    Err(e) => {
                                        error!(
                                            "[PostgreSQL] Streaming query failed: {}, SQL: {}",
                                            Self::postgres_error_message(&e),
                                            sql_preview
                                        );
                                        SqlResult::Error(SqlErrorInfo {
                                            sql: sql_to_execute.to_string(),
                                            message: Self::postgres_error_message(&e),
                                        })
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "[PostgreSQL] Streaming prepare failed: {}, SQL: {}",
                                Self::postgres_error_message(&e),
                                sql_preview
                            );
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql_to_execute.to_string(),
                                message: Self::postgres_error_message(&e),
                            })
                        }
                    };

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    let progress = StreamingProgress::new(current, total, result);
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error && options.stop_on_error {
                        break;
                    }
                }
            }
        }

        debug!("[PostgreSQL] execute_streaming() completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn numeric_bytes(ndigits: i16, weight: i16, sign: u16, scale: u16, digits: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&ndigits.to_be_bytes());
        bytes.extend_from_slice(&weight.to_be_bytes());
        bytes.extend_from_slice(&sign.to_be_bytes());
        bytes.extend_from_slice(&scale.to_be_bytes());
        for digit in digits {
            bytes.extend_from_slice(&digit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn decode_numeric_preserves_precision_and_scale() {
        let raw = numeric_bytes(3, 1, 0x0000, 3, &[1, 2345, 6780]);

        assert_eq!(decode_numeric(&raw).unwrap(), "12345.678");
    }

    #[test]
    fn decode_numeric_preserves_negative_fraction_trailing_zeroes() {
        let raw = numeric_bytes(1, -1, 0x4000, 6, &[1200]);

        assert_eq!(decode_numeric(&raw).unwrap(), "-0.120000");
    }

    #[test]
    fn decode_numeric_preserves_zero_scale() {
        let raw = numeric_bytes(0, 0, 0x0000, 2, &[]);

        assert_eq!(decode_numeric(&raw).unwrap(), "0.00");
    }

    #[test]
    fn decode_numeric_preserves_implicit_base_10000_groups() {
        let large_integer = numeric_bytes(1, 2, 0x0000, 0, &[1]);
        let small_fraction = numeric_bytes(1, -2, 0x0000, 8, &[1234]);

        assert_eq!(decode_numeric(&large_integer).unwrap(), "100000000");
        assert_eq!(decode_numeric(&small_fraction).unwrap(), "0.00001234");
    }

    #[test]
    fn normalize_postgres_compatibility_sql_removes_character_length_suffix() {
        assert_eq!(
            normalize_postgres_compatibility_sql(
                "CREATE TABLE city (a varchar(20 char), b CHARACTER VARYING ( 30 CHAR ), c char(4 char), d character(5 CHAR));"
            ),
            "CREATE TABLE city (a varchar(20), b CHARACTER VARYING ( 30), c char(4), d character(5));"
        );
    }

    #[test]
    fn normalize_postgres_compatibility_sql_preserves_opaque_regions() {
        let sql = r#"
            -- varchar(20 char)
            CREATE TABLE "varchar(20 char)" ("value" varchar(20 byte));
            /* character(10 char) */
            INSERT INTO t VALUES ('varchar(30 char)', $$character varying(40 char)$$);
        "#;

        assert_eq!(normalize_postgres_compatibility_sql(sql), sql);
    }

    #[test]
    fn normalize_postgres_compatibility_sql_requires_character_length_form() {
        let sql = "varchar(20 byte) varchar(20) varchar(20 chars) myvarchar(20 char)";

        assert_eq!(normalize_postgres_compatibility_sql(sql), sql);
    }

    #[derive(Debug)]
    struct DummyVerifier;

    impl ServerCertVerifier for DummyVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, RustlsError> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ECDSA_NISTP256_SHA256]
        }
    }

    fn build_config(extra_params: &[(&str, &str)]) -> DbConnectionConfig {
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::PostgreSQL,
            name: "postgres".to_string(),
            host: "localhost".to_string(),
            port: 5432,
            username: "postgres".to_string(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            credential_reference: None,
            extra_params: extra_params
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn ssl_mode_defaults_to_disable_for_loopback_hosts() {
        let config = build_config(&[]);

        assert_eq!(PostgresDbConnection::ssl_mode(&config), SslMode::Disable);
    }

    #[test]
    fn ssl_mode_defaults_to_prefer_for_non_loopback_hosts() {
        let mut config = build_config(&[]);
        config.host = "db.example.com".to_string();

        assert_eq!(PostgresDbConnection::ssl_mode(&config), SslMode::Prefer);
    }

    #[test]
    fn ssl_mode_honors_disable_and_require() {
        let disable = build_config(&[("ssl_mode", "disable")]);
        let require = build_config(&[("ssl_mode", "require")]);

        assert_eq!(PostgresDbConnection::ssl_mode(&disable), SslMode::Disable);
        assert_eq!(PostgresDbConnection::ssl_mode(&require), SslMode::Require);
    }

    #[test]
    fn ssl_hostname_override_only_ignores_name_errors() {
        let verifier = PostgresServerCertVerifier::new(Arc::new(DummyVerifier), false, true);

        assert!(verifier.should_ignore_certificate_error(&CertificateError::NotValidForName));
        assert!(!verifier.should_ignore_certificate_error(&CertificateError::UnknownIssuer));
    }

    #[test]
    fn ssl_invalid_certs_keep_hostname_validation_by_default() {
        let verifier = PostgresServerCertVerifier::new(Arc::new(DummyVerifier), true, false);

        assert!(verifier.should_ignore_certificate_error(&CertificateError::UnknownIssuer));
        assert!(!verifier.should_ignore_certificate_error(&CertificateError::NotValidForName));
    }

    #[test]
    fn ssl_load_root_certificates_rejects_empty_pem_file() {
        let mut temp_file = NamedTempFile::new().expect("temporary file should be created");
        writeln!(temp_file, "not a certificate").expect("test contents should be written");

        let error = PostgresDbConnection::load_root_certificates(temp_file.path())
            .expect_err("empty PEM file should return an error");

        assert!(
            error
                .to_string()
                .contains("does not contain any certificates"),
            "error message should indicate that the certificate file is empty: {}",
            error
        );
    }

    #[test]
    fn retry_without_tls_matches_peer_certificate_errors() {
        let error = std::io::Error::other(
            "invalid peer certificate: Other(OtherError(UnsupportedCertVersion))",
        );

        assert!(PostgresDbConnection::should_retry_without_tls(&error));
    }

    #[test]
    fn retry_without_tls_ignores_non_tls_errors() {
        let error = std::io::Error::other("password authentication failed for user postgres");

        assert!(!PostgresDbConnection::should_retry_without_tls(&error));
    }

    #[test]
    fn server_error_message_includes_postgres_diagnostics() {
        let message = PostgresDbConnection::format_server_error_message(
            "column \"id\" of relation \"test_no_pk\" contains null values",
            "23502",
            Some("Failing row contains (null, alice)."),
            Some("Backfill the column before adding the primary key."),
            Some("SQL statement \"ALTER TABLE test_no_pk ADD PRIMARY KEY (id)\""),
            Some("public"),
            Some("test_no_pk"),
            Some("id"),
            Some("integer"),
            Some("test_no_pk_pkey"),
        );

        assert_eq!(
            concat!(
                "column \"id\" of relation \"test_no_pk\" contains null values ",
                "(SQLSTATE 23502)\n",
                "Detail: Failing row contains (null, alice).\n",
                "Hint: Backfill the column before adding the primary key.\n",
                "Where: SQL statement \"ALTER TABLE test_no_pk ADD PRIMARY KEY (id)\"\n",
                "Context: schema: public, table: test_no_pk, column: id, datatype: integer, ",
                "constraint: test_no_pk_pkey"
            ),
            message
        );
    }

    #[test]
    fn server_error_message_omits_missing_diagnostics() {
        assert_eq!(
            "could not create unique index (SQLSTATE 23505)",
            PostgresDbConnection::format_server_error_message(
                "could not create unique index",
                "23505",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        );
    }
}
