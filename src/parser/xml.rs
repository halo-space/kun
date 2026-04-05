use crate::parser::query::NodeQuery;
use crate::parser::query::ValueQuery;
use crate::parser::xpath::{evaluate_xml_markup, evaluate_xml_string_values};
use crate::value::Value;

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
            .with_values(
                evaluate_xml_markup(&self.input, &self.node.selector)
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
        evaluate_xml_string_values(&self.input, selector, self.node.trim)
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

    #[test]
    fn xml_query_html_projection_serializes_markup() {
        let query = XmlQuery::new("<items><item id='42'>post</item></items>", "//item");

        assert_eq!(
            query.html().one().as_deref(),
            Some("<item id='42'>post</item>")
        );
    }
}
