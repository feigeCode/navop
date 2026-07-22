const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SourceFingerprint(pub u64);

impl SourceFingerprint {
    pub(crate) fn from_semantics(value: &str) -> Self {
        let mut hash = FNV_OFFSET_BASIS;
        for byte in value.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        Self(hash)
    }
}

pub(crate) fn semantic_node(node: &markdown::mdast::Node) -> SourceFingerprint {
    let mut semantics = String::new();
    write_node(node, &mut semantics);
    SourceFingerprint::from_semantics(&semantics)
}

fn write_node(node: &markdown::mdast::Node, output: &mut String) {
    if let Some(image) = self_linked_image(node) {
        write_node(image, output);
        return;
    }
    output.push_str(node_tag(node));
    output.push('(');
    write_attributes(node, output);
    if let Some(children) = node.children() {
        for child in children {
            write_node(child, output);
        }
    } else {
        write_value(node, output);
    }
    output.push(')');
}

fn self_linked_image(node: &markdown::mdast::Node) -> Option<&markdown::mdast::Node> {
    use markdown::mdast::Node;
    let Node::Link(link) = node else {
        return None;
    };
    let [child] = link.children.as_slice() else {
        return None;
    };
    let Node::Image(image) = child else {
        return None;
    };
    (link.url == image.url && link.title.is_none()).then_some(child)
}

fn node_tag(node: &markdown::mdast::Node) -> &'static str {
    use markdown::mdast::Node;
    match node {
        Node::Root(_) => "root",
        Node::Blockquote(_) => "quote",
        Node::FootnoteDefinition(_) => "footnote-definition",
        Node::List(_) => "list",
        Node::ListItem(_) => "list-item",
        Node::Yaml(_) => "yaml",
        Node::Toml(_) => "toml",
        Node::Break(_) => "break",
        Node::InlineCode(_) => "inline-code",
        Node::InlineMath(_) => "inline-math",
        Node::Delete(_) => "delete",
        Node::Emphasis(_) => "emphasis",
        Node::FootnoteReference(_) => "footnote-reference",
        Node::Html(_) => "html",
        Node::Image(_) => "image",
        Node::ImageReference(_) => "image-reference",
        Node::Link(_) => "link",
        Node::LinkReference(_) => "link-reference",
        Node::Strong(_) => "strong",
        Node::Text(_) => "text",
        Node::Code(_) => "code",
        Node::Math(_) => "math",
        Node::Heading(_) => "heading",
        Node::Table(_) => "table",
        Node::ThematicBreak(_) => "thematic-break",
        Node::TableRow(_) => "table-row",
        Node::TableCell(_) => "table-cell",
        Node::Definition(_) => "definition",
        Node::Paragraph(_) => "paragraph",
        _ => "unsupported",
    }
}

fn write_attributes(node: &markdown::mdast::Node, output: &mut String) {
    use markdown::mdast::Node;
    match node {
        Node::Heading(value) => output.push_str(&value.depth.to_string()),
        Node::List(value) => output.push_str(if value.ordered {
            "ordered"
        } else {
            "unordered"
        }),
        Node::ListItem(value) => output.push_str(&format!("{:?}", value.checked)),
        Node::Code(value) => output.push_str(&format!("{:?}:{:?}", value.lang, value.meta)),
        Node::Link(value) => output.push_str(&format!("{}:{:?}", value.url, value.title)),
        Node::Image(value) => {
            output.push_str(&format!("{}:{}:{:?}", value.alt, value.url, value.title));
        }
        Node::Table(value) => write_table_alignment(&value.align, output),
        _ => {}
    }
}

fn write_table_alignment(alignments: &[markdown::mdast::AlignKind], output: &mut String) {
    use markdown::mdast::AlignKind;
    for alignment in alignments {
        output.push(match alignment {
            AlignKind::None | AlignKind::Left => 'l',
            AlignKind::Center => 'c',
            AlignKind::Right => 'r',
        });
    }
}

fn write_value(node: &markdown::mdast::Node, output: &mut String) {
    use markdown::mdast::Node;
    match node {
        Node::Text(value) => output.push_str(&value.value),
        Node::InlineCode(value) => output.push_str(&value.value),
        Node::InlineMath(value) => output.push_str(&value.value),
        Node::Html(value) => output.push_str(&value.value),
        Node::Code(value) => output.push_str(&value.value),
        Node::Math(value) => output.push_str(&value.value),
        Node::Yaml(value) => output.push_str(&value.value),
        Node::Toml(value) => output.push_str(&value.value),
        Node::Image(value) => output.push_str(&value.url),
        _ => {}
    }
}
