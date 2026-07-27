use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VNode {
    Element(VElement),
    Text(String),
    Fragment(Vec<VNode>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VElement {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub classes: Vec<String>,
    pub children: Vec<VNode>,
}

impl VElement {
    pub fn key(&self) -> Option<&str> {
        self.attrs
            .get("key")
            .or_else(|| self.attrs.get("id"))
            .map(String::as_str)
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    pub fn text_content(&self) -> String {
        self.children.iter().map(VNode::text_content).collect()
    }
}

impl VNode {
    pub fn element(&self) -> Option<&VElement> {
        match self {
            Self::Element(element) => Some(element),
            Self::Text(_) | Self::Fragment(_) => None,
        }
    }

    pub fn text_content(&self) -> String {
        match self {
            Self::Element(element) => element.text_content(),
            Self::Text(text) => text.clone(),
            Self::Fragment(children) => children.iter().map(Self::text_content).collect(),
        }
    }

    pub(crate) fn children_mut(&mut self) -> Option<&mut Vec<VNode>> {
        match self {
            Self::Element(element) => Some(&mut element.children),
            Self::Fragment(children) => Some(children),
            Self::Text(_) => None,
        }
    }
}
