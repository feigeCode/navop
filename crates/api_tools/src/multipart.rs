use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::http::{FieldType, KeyValue};
use crate::variable_resolver::VariableResolver;

#[cfg(test)]
pub fn build_with_boundary(
    rows: &[KeyValue],
    vars: &std::collections::BTreeMap<String, String>,
    boundary: &str,
) -> Result<Vec<u8>> {
    let mut resolver = VariableResolver::new(vars);
    build_with_boundary_with_resolver(rows, &mut resolver, boundary)
}

pub(crate) fn build_with_boundary_with_resolver(
    rows: &[KeyValue],
    resolver: &mut VariableResolver<'_>,
    boundary: &str,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for row in rows
        .iter()
        .filter(|row| row.enabled && !row.key.trim().is_empty())
    {
        append_boundary(&mut body, boundary);
        match row.field_type {
            FieldType::Text => append_text_field(&mut body, row, resolver),
            FieldType::File => append_file_field(&mut body, row, resolver)?,
        }
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(body)
}

fn append_boundary(body: &mut Vec<u8>, boundary: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
}

fn append_text_field(body: &mut Vec<u8>, row: &KeyValue, resolver: &mut VariableResolver<'_>) {
    let name = quote_value(&resolver.substitute(&row.key));
    let value = resolver.substitute(&row.value);
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn append_file_field(
    body: &mut Vec<u8>,
    row: &KeyValue,
    resolver: &mut VariableResolver<'_>,
) -> Result<()> {
    let Some(path) = row
        .file_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
    else {
        bail!("form-data file field '{}' has no file path", row.key);
    };
    let resolved_path = resolver.substitute(path);
    let path = Path::new(&resolved_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .context("form-data file path has no file name")?;
    let bytes =
        std::fs::read(path).with_context(|| format!("read form-data file {resolved_path}"))?;
    let name = quote_value(&resolver.substitute(&row.key));
    let file_name = quote_value(file_name);
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", mime_type(path)).as_bytes());
    body.extend_from_slice(&bytes);
    body.extend_from_slice(b"\r\n");
    Ok(())
}

fn quote_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], "")
}

fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("xml") => "application/xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::http::{FieldType, KeyValue};

    use super::build_with_boundary;

    #[test]
    fn multipart_preserves_binary_files_and_substitutes_text_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("avatar.png");
        let binary = [0_u8, 159, 146, 150, 255];
        std::fs::write(&path, binary).unwrap();
        let rows = vec![
            KeyValue::new("name", "{{user}}"),
            KeyValue {
                key: "avatar".into(),
                value: String::new(),
                enabled: true,
                field_type: FieldType::File,
                file_path: Some(path.to_string_lossy().into_owned()),
            },
        ];

        let body = build_with_boundary(
            &rows,
            &BTreeMap::from([("user".into(), "navop".into())]),
            "test-boundary",
        )
        .unwrap();

        assert!(body.windows(binary.len()).any(|window| window == binary));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"name\"\r\n\r\nnavop\r\n"));
        assert!(text.contains("name=\"avatar\"; filename=\"avatar.png\""));
        assert!(text.contains("Content-Type: image/png"));
        assert!(text.ends_with("--test-boundary--\r\n"));
    }

    #[test]
    fn multipart_reports_missing_file_path() {
        let row = KeyValue {
            key: "avatar".into(),
            value: String::new(),
            enabled: true,
            field_type: FieldType::File,
            file_path: Some("/definitely/missing/avatar.png".into()),
        };

        let error = build_with_boundary(&[row], &BTreeMap::new(), "boundary").unwrap_err();

        assert!(error.to_string().contains("/definitely/missing/avatar.png"));
    }
}
