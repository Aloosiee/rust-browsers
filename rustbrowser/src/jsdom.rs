//! Mutable DOM shared between the engine and JavaScript.
//!
//! Nodes live in an arena and are addressed by index, which keeps JavaScript wrappers cheap and
//! stable. Every mutation bumps [`Dom::version`] so the page knows when to restyle.

use crate::css::{Selector, parse_selector_list};
use crate::dom::{ElementData, Node, NodeType};
use crate::style::matches;
use std::collections::HashMap;

/// Tag used for `DocumentFragment` nodes; inserting one moves its children instead.
pub const FRAGMENT_TAG: &str = "#fragment";

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];

pub struct DNode {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub data: NodeType,
}

/// The live DOM. Node 0 is the document; detached nodes stay in the arena until the page is dropped.
pub struct Dom {
    pub nodes: Vec<DNode>,
    pub version: u64,
}

impl Dom {
    pub fn from_tree(root: &Node) -> Dom {
        let mut dom = Dom {
            nodes: Vec::new(),
            version: 1,
        };
        dom.import(root, None);
        dom
    }

    fn import(&mut self, node: &Node, parent: Option<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(DNode {
            parent,
            children: Vec::new(),
            data: node.node_type.clone(),
        });
        for c in &node.children {
            let cid = self.import(c, Some(id));
            self.nodes[id].children.push(cid);
        }
        id
    }

    /// Snapshots the attached document for styling, flattening fragments.
    pub fn to_tree(&self) -> Node {
        self.export(0)
    }

    fn export(&self, id: usize) -> Node {
        let n = &self.nodes[id];
        let mut children = Vec::with_capacity(n.children.len());
        for &c in &n.children {
            if matches!(&self.nodes[c].data, NodeType::Element(e) if e.tag == FRAGMENT_TAG) {
                for &g in &self.nodes[c].children {
                    children.push(self.export(g));
                }
            } else {
                children.push(self.export(c));
            }
        }
        Node {
            node_type: n.data.clone(),
            children,
            id,
        }
    }

    fn touch(&mut self) {
        self.version += 1;
    }

    pub fn element(&self, id: usize) -> Option<&ElementData> {
        match &self.nodes.get(id)?.data {
            NodeType::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, id: usize) -> Option<&mut ElementData> {
        self.touch();
        match &mut self.nodes.get_mut(id)?.data {
            NodeType::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn is_element(&self, id: usize) -> bool {
        self.element(id).is_some()
    }

    pub fn create_element(&mut self, tag: &str) -> usize {
        self.nodes.push(DNode {
            parent: None,
            children: Vec::new(),
            data: NodeType::Element(ElementData {
                tag: tag.to_ascii_lowercase(),
                attrs: HashMap::new(),
            }),
        });
        self.nodes.len() - 1
    }

    pub fn create_text(&mut self, text: &str) -> usize {
        self.nodes.push(DNode {
            parent: None,
            children: Vec::new(),
            data: NodeType::Text(text.to_string()),
        });
        self.nodes.len() - 1
    }

    pub fn detach(&mut self, id: usize) {
        if let Some(p) = self.nodes[id].parent.take() {
            self.nodes[p].children.retain(|&c| c != id);
            self.touch();
        }
    }

    fn is_ancestor_or_self(&self, maybe_ancestor: usize, id: usize) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == maybe_ancestor {
                return true;
            }
            cur = self.nodes[c].parent;
        }
        false
    }

    /// Inserts `child` before `reference` (or at the end), detaching it from its old parent first.
    pub fn insert_before(&mut self, parent: usize, child: usize, reference: Option<usize>) -> Result<(), String> {
        if self.is_ancestor_or_self(child, parent) {
            return Err("HierarchyRequestError: cannot insert a node into itself".into());
        }
        let moving: Vec<usize> = if matches!(self.element(child), Some(e) if e.tag == FRAGMENT_TAG) {
            std::mem::take(&mut self.nodes[child].children)
        } else {
            self.detach(child);
            vec![child]
        };
        let mut pos = match reference {
            Some(r) => self.nodes[parent]
                .children
                .iter()
                .position(|&c| c == r)
                .ok_or("NotFoundError: reference is not a child")?,
            None => self.nodes[parent].children.len(),
        };
        for m in moving {
            self.nodes[m].parent = Some(parent);
            self.nodes[parent].children.insert(pos, m);
            pos += 1;
        }
        self.touch();
        Ok(())
    }

    pub fn append(&mut self, parent: usize, child: usize) -> Result<(), String> {
        self.insert_before(parent, child, None)
    }

    pub fn clear_children(&mut self, id: usize) {
        for c in std::mem::take(&mut self.nodes[id].children) {
            self.nodes[c].parent = None;
        }
        self.touch();
    }

    pub fn text_content(&self, id: usize) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    fn collect_text(&self, id: usize, out: &mut String) {
        match &self.nodes[id].data {
            NodeType::Text(t) => out.push_str(t),
            _ => {
                for &c in &self.nodes[id].children {
                    self.collect_text(c, out);
                }
            }
        }
    }

    pub fn set_text_content(&mut self, id: usize, text: &str) {
        if let NodeType::Text(t) = &mut self.nodes[id].data {
            *t = text.to_string();
            self.touch();
            return;
        }
        self.clear_children(id);
        if !text.is_empty() {
            let t = self.create_text(text);
            let _ = self.append(id, t);
        }
    }

    /// Parses an HTML fragment into detached nodes.
    pub fn parse_fragment(&mut self, html: &str) -> Vec<usize> {
        let tree = crate::html::parse(&format!("<body>{}</body>", html));
        let body = tree
            .find_first("body")
            .cloned()
            .unwrap_or_else(|| Node::element("body", HashMap::new()));
        body.children.iter().map(|c| self.import(c, None)).collect()
    }

    pub fn set_inner_html(&mut self, id: usize, html: &str) {
        self.clear_children(id);
        for n in self.parse_fragment(html) {
            let _ = self.append(id, n);
        }
    }

    pub fn clone_node(&mut self, id: usize, deep: bool) -> usize {
        let data = self.nodes[id].data.clone();
        self.nodes.push(DNode {
            parent: None,
            children: Vec::new(),
            data,
        });
        let new = self.nodes.len() - 1;
        if deep {
            for c in self.nodes[id].children.clone() {
                let cc = self.clone_node(c, true);
                self.nodes[cc].parent = Some(new);
                self.nodes[new].children.push(cc);
            }
        }
        new
    }

    /// Serializes a node as HTML, escaping text except inside `script` and `style`.
    pub fn serialize(&self, id: usize, out: &mut String) {
        match &self.nodes[id].data {
            NodeType::Text(t) => {
                let raw = self.nodes[id]
                    .parent
                    .and_then(|p| self.element(p))
                    .map_or(false, |e| e.tag == "script" || e.tag == "style");
                if raw {
                    out.push_str(t);
                } else {
                    out.push_str(
                        &t.replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('\u{a0}', "&nbsp;"),
                    );
                }
            }
            NodeType::Element(e) if e.tag == FRAGMENT_TAG => self.serialize_children(id, out),
            NodeType::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                let mut attrs: Vec<_> = e.attrs.iter().collect();
                attrs.sort();
                for (k, v) in attrs {
                    out.push_str(&format!(
                        " {}=\"{}\"",
                        k,
                        v.replace('&', "&amp;").replace('"', "&quot;")
                    ));
                }
                out.push('>');
                if !VOID.contains(&e.tag.as_str()) {
                    self.serialize_children(id, out);
                    out.push_str(&format!("</{}>", e.tag));
                }
            }
            NodeType::Document => self.serialize_children(id, out),
        }
    }

    pub fn serialize_children(&self, id: usize, out: &mut String) {
        for &c in &self.nodes[id].children {
            self.serialize(c, out);
        }
    }

    pub fn ancestors(&self, id: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.nodes[id].parent;
        while let Some(c) = cur {
            out.push(c);
            cur = self.nodes[c].parent;
        }
        out
    }

    pub fn descendants(&self, id: usize, out: &mut Vec<usize>) {
        for &c in &self.nodes[id].children {
            out.push(c);
            self.descendants(c, out);
        }
    }

    pub fn element_matches(&self, id: usize, sels: &[Selector]) -> bool {
        let Some(el) = self.element(id) else { return false };
        let mut chain: Vec<&ElementData> = self
            .ancestors(id)
            .iter()
            .rev()
            .filter_map(|&a| self.element(a))
            .collect();
        chain.retain(|e| e.tag != FRAGMENT_TAG);
        sels.iter().any(|s| matches(s, el, &chain))
    }

    /// Elements under `root` matching a selector list, in document order.
    pub fn query(&self, root: usize, selector: &str, first_only: bool) -> Result<Vec<usize>, String> {
        let sels =
            parse_selector_list(selector).ok_or_else(|| format!("SyntaxError: unsupported selector '{}'", selector))?;
        let mut all = Vec::new();
        self.descendants(root, &mut all);
        let mut out = Vec::new();
        for id in all {
            if self.element_matches(id, &sels) {
                out.push(id);
                if first_only {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub fn find(&self, root: usize, pred: impl Fn(&ElementData) -> bool) -> Vec<usize> {
        let mut all = Vec::new();
        self.descendants(root, &mut all);
        all.into_iter()
            .filter(|&i| self.element(i).map_or(false, &pred))
            .collect()
    }

    pub fn first_tag(&self, tag: &str) -> Option<usize> {
        self.find(0, |e| e.tag == tag).first().copied()
    }

    pub fn closest(&self, id: usize, pred: impl Fn(&ElementData) -> bool) -> Option<usize> {
        std::iter::once(id)
            .chain(self.ancestors(id))
            .find(|&i| self.element(i).map_or(false, &pred))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dom(html: &str) -> Dom {
        Dom::from_tree(&crate::html::parse(html))
    }

    #[test]
    fn round_trips_inner_html() {
        let mut d = dom("<div id=a></div>");
        let div = d.find(0, |e| e.id() == Some("a"))[0];
        d.set_inner_html(div, "<b class=\"x\">1 &amp; 2</b>text");
        let mut out = String::new();
        d.serialize_children(div, &mut out);
        assert_eq!(out, "<b class=\"x\">1 &amp; 2</b>text");
    }

    #[test]
    fn insertion_moves_nodes_and_bumps_version() {
        let mut d = dom("<ul><li>1</li><li>2</li></ul>");
        let ul = d.first_tag("ul").unwrap();
        let first = d.nodes[ul].children[0];
        let before = d.version;
        d.append(ul, first).unwrap();
        assert_eq!(d.text_content(ul), "21");
        assert!(d.version > before);
    }

    #[test]
    fn refuses_to_insert_a_node_into_itself() {
        let mut d = dom("<div><p></p></div>");
        let div = d.first_tag("div").unwrap();
        let p = d.first_tag("p").unwrap();
        assert!(d.append(p, div).is_err());
    }

    #[test]
    fn fragments_insert_their_children() {
        let mut d = dom("<ul></ul>");
        let ul = d.first_tag("ul").unwrap();
        let frag = d.create_element(FRAGMENT_TAG);
        for t in ["a", "b"] {
            let li = d.create_element("li");
            let text = d.create_text(t);
            d.append(li, text).unwrap();
            d.append(frag, li).unwrap();
        }
        d.append(ul, frag).unwrap();
        assert_eq!(d.nodes[ul].children.len(), 2);
        assert!(d.nodes[frag].children.is_empty());
    }

    #[test]
    fn queries_with_selectors() {
        let d = dom("<div class=a><p>1</p></div><p class=a>2</p>");
        let hits = d.query(0, ".a p, p.a", false).unwrap();
        let texts: Vec<String> = hits.iter().map(|&i| d.text_content(i)).collect();
        assert_eq!(texts, ["1", "2"]);
        assert!(d.query(0, "p:first-child", false).is_err());
    }
}
