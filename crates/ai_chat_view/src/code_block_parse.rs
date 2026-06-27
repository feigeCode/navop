#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FencedCodeBlock {
    pub language: Option<String>,
    pub code: String,
    pub start: usize,
    pub end: usize,
}

pub fn extract_fenced_code_blocks(markdown: &str) -> Vec<FencedCodeBlock> {
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = markdown[cursor..].find("```") {
        let start = cursor + relative_start;
        let info_start = start + 3;
        let Some(info_end) = markdown[info_start..]
            .find('\n')
            .map(|offset| info_start + offset)
        else {
            break;
        };
        let code_start = info_end + 1;
        let Some(relative_end) = markdown[code_start..].find("\n```") else {
            break;
        };
        let code_end = code_start + relative_end + 1;
        let end = code_end + 3;
        let info = markdown[info_start..info_end].trim();
        blocks.push(FencedCodeBlock {
            language: first_info_token(info),
            code: markdown[code_start..code_end].to_string(),
            start,
            end,
        });
        cursor = end;
    }
    blocks
}

fn first_info_token(info: &str) -> Option<String> {
    info.split_whitespace()
        .next()
        .filter(|lang| !lang.is_empty())
        .map(|lang| lang.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fenced_code_blocks_with_language_and_range() {
        let markdown = "before\n```sql\nselect 1;\n```\nafter";

        let blocks = extract_fenced_code_blocks(markdown);

        assert_eq!(1, blocks.len());
        assert_eq!(Some("sql"), blocks[0].language.as_deref());
        assert_eq!("select 1;\n", blocks[0].code);
        assert_eq!(
            "```sql\nselect 1;\n```",
            &markdown[blocks[0].start..blocks[0].end]
        );
    }

    #[test]
    fn extracts_multiple_fenced_code_blocks_and_ignores_unclosed() {
        let markdown = "```json\n{\"a\":1}\n```\ntext\n```bash\necho hi\n```\n```sql\nselect";

        let blocks = extract_fenced_code_blocks(markdown);

        assert_eq!(2, blocks.len());
        assert_eq!(Some("json"), blocks[0].language.as_deref());
        assert_eq!(Some("bash"), blocks[1].language.as_deref());
    }

    #[test]
    fn fenced_code_language_uses_first_info_token() {
        let markdown = "```chart-json title=\"x\"\n{\"chart_type\":\"bar\",\"data\":[]}\n```";

        let blocks = extract_fenced_code_blocks(markdown);

        assert_eq!(Some("chart-json"), blocks[0].language.as_deref());
    }
}
