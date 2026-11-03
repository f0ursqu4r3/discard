//! The declarative element tree that plugins return from `render` functions.
//! A Lua table `{ "row", gap = 8, { "text", "hi" } }` becomes an `Element`. Rust validates it.

use mlua::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_NODES: usize = 2000;
pub const MAX_DEPTH: usize = 32;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tag {
    Row,
    Column,
    Text,
    Markdown,
    Button,
    Avatar,
    Badge,
    Divider,
    Spacer,
    Dot,
}

impl Tag {
    /// Every tag, in documentation order.
    pub const ALL: [Tag; 10] =
        [Tag::Row, Tag::Column, Tag::Text, Tag::Markdown, Tag::Button, Tag::Avatar, Tag::Badge, Tag::Divider, Tag::Spacer, Tag::Dot];

    pub fn name(self) -> &'static str {
        match self {
            Tag::Row => "row",
            Tag::Column => "column",
            Tag::Text => "text",
            Tag::Markdown => "markdown",
            Tag::Button => "button",
            Tag::Avatar => "avatar",
            Tag::Badge => "badge",
            Tag::Divider => "divider",
            Tag::Spacer => "spacer",
            Tag::Dot => "dot",
        }
    }

    pub fn parse(s: &str) -> Option<Tag> {
        Some(match s {
            "row" => Tag::Row,
            "column" => Tag::Column,
            "text" => Tag::Text,
            "markdown" => Tag::Markdown,
            "button" => Tag::Button,
            "avatar" => Tag::Avatar,
            "badge" => Tag::Badge,
            "divider" => Tag::Divider,
            "spacer" => Tag::Spacer,
            "dot" => Tag::Dot,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prop {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub tag: Tag,
    pub props: BTreeMap<String, Prop>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Text(String),
    Element(Element),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ElementError {
    #[error("element must be a table")]
    NotATable,
    #[error("element needs a tag as its first entry")]
    MissingTag,
    #[error("unknown tag `{0}`")]
    UnknownTag(String),
    #[error("property `{0}` has an unsupported type")]
    BadProp(String),
    #[error("child {0} is not an element or a string")]
    BadChild(usize),
    #[error("tree is deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("tree has more than {MAX_NODES} nodes")]
    TooManyNodes,
    #[error("text is longer than {MAX_TEXT_BYTES} bytes")]
    TextTooLong,
}

/// Convert a Lua value to a validated element.
pub fn from_lua(value: &Value) -> Result<Element, ElementError> {
    let mut nodes = 0usize;
    element(value, 1, &mut nodes)
}

fn check_text(s: &str) -> Result<(), ElementError> {
    if s.len() > MAX_TEXT_BYTES { Err(ElementError::TextTooLong) } else { Ok(()) }
}

fn element(value: &Value, depth: usize, nodes: &mut usize) -> Result<Element, ElementError> {
    if depth > MAX_DEPTH {
        return Err(ElementError::TooDeep);
    }
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err(ElementError::TooManyNodes);
    }
    let Value::Table(table) = value else { return Err(ElementError::NotATable) };

    let tag = match table.raw_get::<Value>(1) {
        Ok(Value::String(s)) => {
            let name = s.to_string_lossy().to_string();
            Tag::parse(&name).ok_or(ElementError::UnknownTag(name))?
        }
        _ => return Err(ElementError::MissingTag),
    };

    let mut props = BTreeMap::new();
    for pair in table.pairs::<Value, Value>() {
        let Ok((key, val)) = pair else { continue };
        let Value::String(key) = key else { continue };
        let key = key.to_string_lossy().to_string();
        let prop = match val {
            Value::Boolean(b) => Prop::Bool(b),
            Value::Integer(i) => Prop::Number(i as f64),
            Value::Number(n) => Prop::Number(n),
            Value::String(s) => {
                let text = s.to_string_lossy().to_string();
                check_text(&text)?;
                Prop::Text(text)
            }
            _ => return Err(ElementError::BadProp(key)),
        };
        props.insert(key, prop);
    }

    let mut children = Vec::new();
    for index in 2..=table.raw_len() {
        let child: Value = table.raw_get(index).map_err(|_| ElementError::BadChild(index - 2))?;
        let position = index - 2;
        children.push(match child {
            Value::String(s) => {
                let text = s.to_string_lossy().to_string();
                check_text(&text)?;
                Node::Text(text)
            }
            Value::Integer(i) => Node::Text(i.to_string()),
            Value::Number(n) => Node::Text(n.to_string()),
            Value::Table(_) => Node::Element(element(&child, depth + 1, nodes)?),
            _ => return Err(ElementError::BadChild(position)),
        });
    }
    Ok(Element { tag, props, children })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn eval(src: &str) -> Result<Element, ElementError> {
        let lua = Lua::new();
        let v: Value = lua.load(src).eval().unwrap();
        from_lua(&v)
    }

    #[test]
    fn a_tag_props_and_children_become_an_element() {
        let e = eval(r#"return { "row", gap = 8, visible = true, id = "x", { "text", "hi" }, "plain" }"#).unwrap();
        assert_eq!(e.tag, Tag::Row);
        assert_eq!(e.props["gap"], Prop::Number(8.0));
        assert_eq!(e.props["visible"], Prop::Bool(true));
        assert_eq!(e.props["id"], Prop::Text("x".into()));
        assert_eq!(e.children.len(), 2);
        assert_eq!(e.children[1], Node::Text("plain".into()));
        let Node::Element(inner) = &e.children[0] else { panic!() };
        assert_eq!(inner.tag, Tag::Text);
        assert_eq!(inner.children, vec![Node::Text("hi".into())]);
    }

    #[test]
    fn children_keep_their_order() {
        let e = eval(r#"return { "column", "a", "b", "c" }"#).unwrap();
        let texts: Vec<_> = e.children.iter().map(|n| match n { Node::Text(t) => t.clone(), _ => String::new() }).collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    #[test]
    fn numbers_as_children_are_rendered_as_text() {
        let e = eval(r#"return { "text", 42 }"#).unwrap();
        assert_eq!(e.children, vec![Node::Text("42".into())]);
    }

    #[test]
    fn rejects_non_tables_missing_tags_and_unknown_tags() {
        assert_eq!(eval("return 5"), Err(ElementError::NotATable));
        assert_eq!(eval(r#"return { gap = 1 }"#), Err(ElementError::MissingTag));
        assert_eq!(eval(r#"return { "script" }"#), Err(ElementError::UnknownTag("script".into())));
    }

    #[test]
    fn rejects_functions_and_tables_as_property_values() {
        assert_eq!(eval(r#"return { "row", on = function() end }"#), Err(ElementError::BadProp("on".into())));
        assert_eq!(eval(r#"return { "row", style = {} }"#), Err(ElementError::BadProp("style".into())));
    }

    #[test]
    fn rejects_bad_children() {
        assert_eq!(eval(r#"return { "row", true }"#), Err(ElementError::BadChild(0)));
        assert_eq!(eval(r#"return { "row", "ok", function() end }"#), Err(ElementError::BadChild(1)));
    }

    #[test]
    fn limits_depth() {
        let deep = "return ".to_string() + &"{ \"row\", ".repeat(40) + &"}".repeat(40);
        assert_eq!(eval(&deep), Err(ElementError::TooDeep));
        let ok = "return ".to_string() + &"{ \"row\", ".repeat(20) + &"}".repeat(20);
        assert!(eval(&ok).is_ok());
    }

    #[test]
    fn limits_node_count() {
        let many = format!("local t = {{ \"column\" }} for i = 1, {} do t[#t + 1] = {{ \"text\", \"x\" }} end return t", MAX_NODES + 5);
        assert_eq!(eval(&many), Err(ElementError::TooManyNodes));
    }

    #[test]
    fn limits_text_size() {
        let big = format!("return {{ \"text\", string.rep(\"a\", {}) }}", MAX_TEXT_BYTES + 1);
        assert_eq!(eval(&big), Err(ElementError::TextTooLong));
    }

    #[test]
    fn cyclic_tables_hit_the_depth_limit_instead_of_looping() {
        assert_eq!(eval(r#"local t = { "row" } t[2] = t return t"#), Err(ElementError::TooDeep));
    }
}
