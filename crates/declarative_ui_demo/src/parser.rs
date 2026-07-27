use std::collections::BTreeMap;

use html5ever::{QualName, local_name, namespace_url, ns, parse_fragment, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use thiserror::Error;

use crate::{CompileLimits, ParseResource, VElement, VNode, html_source::expand_self_closing_tags};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HtmlParseError {
    #[error("the HTML fragment contains no renderable nodes")]
    EmptyFragment,
    #[error("element <{tag}> is not allowed")]
    ForbiddenElement { tag: String },
    #[error("attribute `{name}` is not allowed")]
    ForbiddenAttribute { name: String },
    #[error("{resource} limit exceeded: limit {limit}, observed {actual}")]
    ResourceLimitExceeded {
        resource: ParseResource,
        limit: usize,
        actual: usize,
    },
}

pub fn parse_html(source: &str) -> Result<VNode, HtmlParseError> {
    parse_html_with_limits(source, CompileLimits::default())
}

pub fn parse_html_with_limits(
    source: &str,
    limits: CompileLimits,
) -> Result<VNode, HtmlParseError> {
    enforce_limit(
        ParseResource::SourceBytes,
        source.len(),
        limits.max_source_bytes,
    )?;
    let context = QualName::new(None, ns!(html), local_name!("div"));
    let normalized = expand_self_closing_tags(source);
    let dom = parse_fragment(RcDom::default(), Default::default(), context, vec![])
        .one(normalized.as_ref());
    let container = fragment_container(&dom.document);
    let nodes = VNodeConverter::new(limits).convert_children(&container, 0)?;
    normalize_root(nodes)
}

fn normalize_root(mut nodes: Vec<VNode>) -> Result<VNode, HtmlParseError> {
    match nodes.len() {
        0 => Err(HtmlParseError::EmptyFragment),
        1 => Ok(nodes.remove(0)),
        _ => Ok(VNode::Fragment(nodes)),
    }
}

fn fragment_container(document: &Handle) -> Handle {
    let children = document.children.borrow();
    match children.as_slice() {
        [child] if is_html_element(child) => child.clone(),
        _ => document.clone(),
    }
}

fn is_html_element(handle: &Handle) -> bool {
    matches!(
        &handle.data,
        NodeData::Element { name, .. } if name.local.as_ref() == "html"
    )
}

struct VNodeConverter {
    limits: CompileLimits,
    nodes: ResourceCounter,
    attributes: ResourceCounter,
    classes: ResourceCounter,
}

impl VNodeConverter {
    fn new(limits: CompileLimits) -> Self {
        Self {
            limits,
            nodes: ResourceCounter::new(ParseResource::Nodes, limits.max_nodes),
            attributes: ResourceCounter::new(ParseResource::Attributes, limits.max_attributes),
            classes: ResourceCounter::new(ParseResource::Classes, limits.max_classes),
        }
    }

    fn convert_children(
        &mut self,
        handle: &Handle,
        parent_depth: usize,
    ) -> Result<Vec<VNode>, HtmlParseError> {
        let mut nodes = Vec::new();
        for child in handle.children.borrow().iter() {
            if let Some(node) = self.convert_node(child, parent_depth)? {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    fn convert_node(
        &mut self,
        handle: &Handle,
        parent_depth: usize,
    ) -> Result<Option<VNode>, HtmlParseError> {
        match &handle.data {
            NodeData::Document => {
                let children = self.convert_children(handle, parent_depth)?;
                Ok(Some(VNode::Fragment(children)))
            }
            NodeData::Element { name, attrs, .. } => self
                .convert_element(ElementSource {
                    tag: name.local.as_ref(),
                    raw_attrs: attrs.borrow().as_slice(),
                    handle,
                    parent_depth,
                })
                .map(Some),
            NodeData::Text { contents } => self.convert_text(contents.borrow().as_ref()),
            NodeData::Comment { .. }
            | NodeData::Doctype { .. }
            | NodeData::ProcessingInstruction { .. } => Ok(None),
        }
    }

    fn convert_element(&mut self, source: ElementSource<'_>) -> Result<VNode, HtmlParseError> {
        validate_element(source.tag)?;
        let depth = source.parent_depth + 1;
        enforce_limit(ParseResource::Depth, depth, self.limits.max_depth)?;
        self.nodes.consume(1)?;
        let (attrs, classes) = self.convert_attributes(source.raw_attrs)?;
        let children = self.convert_children(source.handle, depth)?;
        Ok(VNode::Element(VElement {
            tag: source.tag.to_owned(),
            attrs,
            classes,
            children,
        }))
    }

    fn convert_text(&mut self, text: &str) -> Result<Option<VNode>, HtmlParseError> {
        let Some(text) = normalize_text(text) else {
            return Ok(None);
        };
        self.nodes.consume(1)?;
        Ok(Some(VNode::Text(text)))
    }

    fn convert_attributes(
        &mut self,
        raw_attrs: &[html5ever::Attribute],
    ) -> Result<(BTreeMap<String, String>, Vec<String>), HtmlParseError> {
        self.attributes.consume(raw_attrs.len())?;
        let mut attrs = BTreeMap::new();
        let mut classes = Vec::new();
        for attr in raw_attrs {
            let name = attr.name.local.to_string();
            validate_attribute(&name)?;
            if name == "class" {
                self.convert_classes(attr.value.as_ref(), &mut classes)?;
            } else {
                attrs.insert(name, attr.value.to_string());
            }
        }
        Ok((attrs, classes))
    }

    fn convert_classes(
        &mut self,
        value: &str,
        classes: &mut Vec<String>,
    ) -> Result<(), HtmlParseError> {
        let parsed = value
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.classes.consume(parsed.len())?;
        classes.extend(parsed);
        Ok(())
    }
}

struct ElementSource<'a> {
    tag: &'a str,
    raw_attrs: &'a [html5ever::Attribute],
    handle: &'a Handle,
    parent_depth: usize,
}

struct ResourceCounter {
    resource: ParseResource,
    limit: usize,
    current: usize,
}

impl ResourceCounter {
    fn new(resource: ParseResource, limit: usize) -> Self {
        Self {
            resource,
            limit,
            current: 0,
        }
    }

    fn consume(&mut self, increment: usize) -> Result<(), HtmlParseError> {
        let actual = self.current.saturating_add(increment);
        enforce_limit(self.resource, actual, self.limit)?;
        self.current = actual;
        Ok(())
    }
}

fn enforce_limit(
    resource: ParseResource,
    actual: usize,
    limit: usize,
) -> Result<(), HtmlParseError> {
    if actual > limit {
        return Err(HtmlParseError::ResourceLimitExceeded {
            resource,
            limit,
            actual,
        });
    }
    Ok(())
}

fn validate_element(tag: &str) -> Result<(), HtmlParseError> {
    if matches!(tag, "script" | "style") {
        return Err(HtmlParseError::ForbiddenElement {
            tag: tag.to_owned(),
        });
    }
    Ok(())
}

fn validate_attribute(name: &str) -> Result<(), HtmlParseError> {
    if name == "style" || name.starts_with("on") {
        return Err(HtmlParseError::ForbiddenAttribute {
            name: name.to_owned(),
        });
    }
    Ok(())
}

fn normalize_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}
