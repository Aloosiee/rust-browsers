//! Immutable document tree consumed by the style system.
//!
//! The tree is a snapshot of [`crate::jsdom::Dom`]; each node keeps the arena id it came from
//! so layout results (hit regions, element rectangles) can be mapped back to live DOM nodes.

use std::collections::HashMap;

/// Tag name and attributes of an element. Tag and attribute names are lowercase.
#[derive(Debug, Clone)]
pub struct ElementData {
    pub tag: String,
    pub attrs: HashMap<String, String>,
}

/// The kind of a DOM node.
#[derive(Debug, Clone)]
pub enum NodeType {
    Document,
    Element(ElementData),
    Text(String),
}

/// A node in the document snapshot. `id` is the node's index in the live DOM arena.
#[derive(Debug, Clone)]
pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
    pub id: usize,
}

impl ElementData {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(|s| s.as_str())
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class").unwrap_or("").split_ascii_whitespace()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes().any(|c| c == class)
    }
}

impl Node {
    pub fn element(tag: &str, attrs: HashMap<String, String>) -> Node {
        Node {
            node_type: NodeType::Element(ElementData {
                tag: tag.to_string(),
                attrs,
            }),
            children: Vec::new(),
            id: 0,
        }
    }

    pub fn text(s: String) -> Node {
        Node {
            node_type: NodeType::Text(s),
            children: Vec::new(),
            id: 0,
        }
    }

    pub fn as_element(&self) -> Option<&ElementData> {
        match &self.node_type {
            NodeType::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn tag(&self) -> Option<&str> {
        self.as_element().map(|e| e.tag.as_str())
    }

    /// Concatenated text of this node and its descendants.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        match &self.node_type {
            NodeType::Text(t) => out.push_str(t),
            _ => self.children.iter().for_each(|c| c.collect_text(out)),
        }
    }

    pub fn find_all<'a>(&'a self, tag: &str, out: &mut Vec<&'a Node>) {
        if self.tag() == Some(tag) {
            out.push(self);
        }
        for c in &self.children {
            c.find_all(tag, out);
        }
    }

    /// Depth-first search for the first element with the given tag.
    pub fn find_first(&self, tag: &str) -> Option<&Node> {
        if self.tag() == Some(tag) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find_first(tag))
    }
}
