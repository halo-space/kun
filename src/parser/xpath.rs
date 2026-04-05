use crate::parser::query::NodeQuery;
use crate::parser::query::{ValueQuery, trim_text};
use crate::value::Value;
use ego_tree::NodeRef;
use scraper::{Html, Node as HtmlNode};
use xrust::item::{Item, Node as XrustNode, Sequence};
use xrust::parser::ParseError;
use xrust::transform::context::{ContextBuilder, StaticContextBuilder};
use xrust::trees::smite::RNode;
use xrust::xdmerror::{Error as XrustError, ErrorKind as XrustErrorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Html,
    Xml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Projection {
    StringValue,
    Xml,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XPathQuery {
    pub node: NodeQuery,
    pub input: String,
}

impl XPathQuery {
    pub fn new(input: impl Into<String>, selector: impl Into<String>) -> Self {
        Self {
            node: NodeQuery::new(selector),
            input: input.into(),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.extract(&self.node.selector).into_iter().next()
    }

    pub fn all(&self) -> Vec<String> {
        self.extract(&self.node.selector)
    }

    pub fn text(&self) -> ValueQuery {
        ValueQuery::new(crate::parser::Kind::Text, self.node.selector.clone())
            .with_trim(self.node.trim)
            .with_values(
                self.extract(&format!("{}//text()", self.node.selector))
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            )
    }

    pub fn html(&self) -> ValueQuery {
        ValueQuery::new(crate::parser::Kind::Html, self.node.selector.clone())
            .with_trim(false)
            .with_values(
                evaluate_xpath(
                    &self.input,
                    &self.node.selector,
                    false,
                    SourceKind::Html,
                    Projection::Xml,
                )
                .into_iter()
                .map(Value::String)
                .collect(),
            )
    }

    pub fn attr(&self, name: impl Into<String>) -> ValueQuery {
        let name = name.into();
        ValueQuery::new(
            crate::parser::Kind::Attribute,
            format!("{}//@{name}", self.node.selector),
        )
        .with_trim(self.node.trim)
        .with_values(
            self.extract(&format!("{}//@{name}", self.node.selector))
                .into_iter()
                .map(Value::String)
                .collect(),
        )
    }

    fn extract(&self, selector: &str) -> Vec<String> {
        evaluate_xpath(
            &self.input,
            selector,
            self.node.trim,
            SourceKind::Html,
            Projection::StringValue,
        )
    }
}

pub(crate) fn evaluate_xml_string_values(input: &str, selector: &str, trim: bool) -> Vec<String> {
    evaluate_xpath(
        input,
        selector,
        trim,
        SourceKind::Xml,
        Projection::StringValue,
    )
}

pub(crate) fn evaluate_xml_markup(input: &str, selector: &str) -> Vec<String> {
    evaluate_xpath(input, selector, false, SourceKind::Xml, Projection::Xml)
}

fn evaluate_xpath(
    input: &str,
    selector: &str,
    trim: bool,
    source_kind: SourceKind,
    projection: Projection,
) -> Vec<String> {
    let Some(sequence) = evaluate_xpath_sequence(input, selector, source_kind) else {
        return Vec::new();
    };

    sequence
        .into_iter()
        .map(|item| match projection {
            Projection::StringValue => trim_text(&item.to_string(), trim),
            Projection::Xml => item.to_xml(),
        })
        .collect()
}

fn evaluate_xpath_sequence(
    input: &str,
    selector: &str,
    source_kind: SourceKind,
) -> Option<Sequence<RNode>> {
    let source = match source_kind {
        SourceKind::Html => normalize_html_document(input),
        SourceKind::Xml => input.to_string(),
    };

    let document = RNode::new_document();
    xrust::parser::xml::parse(
        document.clone(),
        &source,
        Some(|_: &_| Err(ParseError::MissingNameSpace)),
    )
    .ok()?;

    let transform = xrust::parser::xpath::parse(selector, Some(document.clone()), None).ok()?;
    let context = ContextBuilder::new()
        .context(vec![Item::Node(document)])
        .build();
    let mut static_context = StaticContextBuilder::new()
        .message(|_| Ok(()))
        .fetcher(|_| Ok(String::new()))
        .parser(|_| {
            Err(XrustError::new(
                XrustErrorKind::NotImplemented,
                "not implemented",
            ))
        })
        .build();

    context.dispatch(&mut static_context, &transform).ok()
}

fn normalize_html_document(input: &str) -> String {
    let document = Html::parse_document(input);
    let mut output = String::from("<document>");
    for child in document.tree.root().children() {
        serialize_html_node(child, &mut output);
    }
    output.push_str("</document>");
    output
}

fn serialize_html_node(node: NodeRef<'_, HtmlNode>, output: &mut String) {
    match node.value() {
        HtmlNode::Document | HtmlNode::Fragment | HtmlNode::Doctype(_) => {
            for child in node.children() {
                serialize_html_node(child, output);
            }
        }
        HtmlNode::Comment(_) | HtmlNode::ProcessingInstruction(_) => {}
        HtmlNode::Text(text) => {
            output.push_str(&escape_xml_text(text.as_ref()));
        }
        HtmlNode::Element(element) => {
            output.push('<');
            output.push_str(element.name());

            for (name, value) in element.attrs() {
                output.push(' ');
                output.push_str(name);
                output.push_str("=\"");
                output.push_str(&escape_xml_attribute(value));
                output.push('"');
            }

            let children = node.children().collect::<Vec<_>>();
            if children.is_empty() {
                output.push_str("/>");
                return;
            }

            output.push('>');
            for child in children {
                serialize_html_node(child, output);
            }
            output.push_str("</");
            output.push_str(element.name());
            output.push('>');
        }
    }
}

fn escape_xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attribute(value: &str) -> String {
    escape_xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpath_query_reads_text_from_xml_like_markup() {
        let query = XPathQuery::new("<root><h1>Hello</h1></root>", "//h1");

        assert_eq!(query.one().as_deref(), Some("Hello"));
        assert_eq!(query.text().one().as_deref(), Some("Hello"));
    }

    #[test]
    fn xpath_query_reads_malformed_html_documents() {
        let query = XPathQuery::new("<html><body><div><h1>Hello</h1><p>world", "//body/div/h1");

        assert_eq!(query.one().as_deref(), Some("Hello"));
    }

    #[test]
    fn xpath_query_html_projection_serializes_markup() {
        let query = XPathQuery::new("<div><h1>Hello</h1></div>", "//h1");

        assert_eq!(query.html().one().as_deref(), Some("<h1>Hello</h1>"));
    }
}
