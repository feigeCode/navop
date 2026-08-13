use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};

use crate::executor::QueryResult;

#[derive(Debug)]
pub(super) enum ImportedValue {
    Null,
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug)]
pub(super) struct ImportedRow {
    pub fields: Vec<(String, ImportedValue)>,
}

pub(super) fn parse_rows(
    data: &str,
    target_table: &str,
    legacy_table_tag: &str,
) -> Result<Vec<ImportedRow>> {
    let document = roxmltree::Document::parse(data)
        .map_err(|error| anyhow!("Failed to parse XML: {error}"))?;
    let root = document.root_element();
    if root.tag_name().name() != "data" {
        bail!("XML root element must be <data>");
    }

    root.children()
        .filter(|node| node.is_element())
        .filter(|node| is_target_row(*node, target_table, legacy_table_tag))
        .map(parse_row)
        .collect()
}

pub(super) fn serialize_table(table: &str, result: &QueryResult) -> Result<String> {
    let binary_cells: HashMap<(usize, usize), &[u8]> = result
        .binary_cells
        .iter()
        .map(|cell| ((cell.row_index, cell.column_index), cell.bytes.as_slice()))
        .collect();
    let mut output = String::new();

    for (row_index, row) in result.rows.iter().enumerate() {
        output.push_str("  <row table=\"");
        output.push_str(&escape_xml(table)?);
        output.push_str("\">\n");

        for (column_index, column) in result.columns.iter().enumerate() {
            output.push_str("    <field name=\"");
            output.push_str(&escape_xml(column)?);
            output.push('"');

            if let Some(bytes) = binary_cells.get(&(row_index, column_index)) {
                output.push_str(" encoding=\"hex\">");
                output.push_str(&hex::encode(bytes));
            } else {
                match row.get(column_index).and_then(Option::as_deref) {
                    None => output.push_str(" null=\"true\">"),
                    Some(value) => {
                        output.push('>');
                        output.push_str(&escape_xml(value)?);
                    }
                }
            }
            output.push_str("</field>\n");
        }

        output.push_str("  </row>\n");
    }

    Ok(output)
}

fn is_target_row(
    node: roxmltree::Node<'_, '_>,
    target_table: &str,
    legacy_table_tag: &str,
) -> bool {
    if node.tag_name().name() == "row" {
        return node.attribute("table") == Some(target_table);
    }
    node.tag_name().name() == legacy_table_tag || node.attribute("name") == Some(target_table)
}

fn parse_row(node: roxmltree::Node<'_, '_>) -> Result<ImportedRow> {
    let new_format = node.tag_name().name() == "row";
    let mut names = HashSet::new();
    let mut fields = Vec::new();

    for field in node.children().filter(|child| child.is_element()) {
        let column = if new_format {
            if field.tag_name().name() != "field" {
                bail!("XML <row> may only contain <field> elements");
            }
            field
                .attribute("name")
                .filter(|name| !name.is_empty())
                .ok_or_else(|| anyhow!("XML <field> is missing a non-empty name attribute"))?
        } else {
            field.attribute("name").unwrap_or(field.tag_name().name())
        };

        if !names.insert(column.to_string()) {
            bail!("XML row contains duplicate column {column:?}");
        }
        fields.push((column.to_string(), parse_value(field)?));
    }

    if fields.is_empty() {
        bail!("XML row does not contain any fields");
    }
    Ok(ImportedRow { fields })
}

fn parse_value(field: roxmltree::Node<'_, '_>) -> Result<ImportedValue> {
    if field.children().any(|child| child.is_element()) {
        bail!("XML field values cannot contain nested elements");
    }
    let text = field.text().unwrap_or("");
    let is_null = match field.attribute("null") {
        None | Some("false") => false,
        Some("true") => true,
        Some(value) => bail!("Invalid XML null attribute value {value:?}"),
    };

    if is_null {
        if field.attribute("encoding").is_some() || !text.is_empty() {
            bail!("A null XML field cannot also contain encoded or textual data");
        }
        return Ok(ImportedValue::Null);
    }

    match field.attribute("encoding") {
        None => Ok(ImportedValue::Text(text.to_string())),
        Some("hex") => hex::decode(text.trim())
            .map(ImportedValue::Binary)
            .map_err(|error| anyhow!("Invalid hexadecimal XML field: {error}")),
        Some(encoding) => bail!("Unsupported XML field encoding {encoding:?}"),
    }
}

fn escape_xml(value: &str) -> Result<String> {
    if let Some(character) = value
        .chars()
        .find(|character| !is_valid_xml_char(*character))
    {
        bail!(
            "Value contains character U+{:04X}, which is not valid in XML 1.0",
            character as u32
        );
    }

    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

fn is_valid_xml_char(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || matches!(character as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

#[cfg(test)]
mod tests {
    use super::escape_xml;

    #[test]
    fn escape_xml_rejects_xml_1_0_control_characters() {
        let error = escape_xml("before\0after").expect_err("NUL should be rejected");

        assert!(error.to_string().contains("U+0000"));
    }
}
