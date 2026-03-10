use crate::parser::query::NodeQuery;
use crate::parser::query::{ValueQuery, trim_text};
use crate::value::Value;
use sxd_document::parser;
use sxd_xpath::{Context, Factory, Value as XPathValue};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlQuery {
    pub node: NodeQuery,
    pub input: String,
}

impl XmlQuery {
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
            .with_values(self.extract(&self.node.selector).into_iter().map(Value::String).collect())
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
        let Ok(package) = parser::parse(&self.input) else {
            return Vec::new();
        };
        let document = package.as_document();
        let factory = Factory::new();
        let Ok(Some(xpath)) = factory.build(selector) else {
            return Vec::new();
        };
        let context = Context::new();
        let Ok(value) = xpath.evaluate(&context, document.root()) else {
            return Vec::new();
        };

        match value {
            XPathValue::Nodeset(nodes) => nodes
                .document_order()
                .into_iter()
                .map(|node| trim_text(&node.string_value(), self.node.trim))
                .collect(),
            XPathValue::String(value) => vec![trim_text(&value, self.node.trim)],
            XPathValue::Boolean(value) => vec![value.to_string()],
            XPathValue::Number(value) => vec![value.to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_query_reads_attribute_values() {
        let query = XmlQuery::new("<items><item id='42'>post</item></items>", "//item");

        assert_eq!(query.attr("id").one().as_deref(), Some("42"));
    }
}
