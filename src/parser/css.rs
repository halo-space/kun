use crate::parser::query::{NodeQuery, ValueQuery, trim_text};
use crate::value::Value;
use scraper::{ElementRef, Html, Selector};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CssQuery {
    pub node: NodeQuery,
    pub input: String,
}

impl CssQuery {
    pub fn new(input: impl Into<String>, selector: impl Into<String>) -> Self {
        Self {
            node: NodeQuery::new(selector),
            input: input.into(),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.extract().into_iter().next()
    }

    pub fn all(&self) -> Vec<String> {
        self.extract()
    }

    pub fn text(&self) -> ValueQuery {
        ValueQuery::new(crate::parser::Kind::Text, self.node.selector.clone())
            .with_trim(self.node.trim)
            .with_values(self.map_base(|element| {
                Value::String(trim_text(&node_text(&element), self.node.trim))
            }))
    }

    pub fn html(&self) -> ValueQuery {
        ValueQuery::new(crate::parser::Kind::Html, self.node.selector.clone())
            .with_trim(false)
            .with_values(self.map_base(|element| Value::String(element.html())))
    }

    pub fn attr(&self, name: impl Into<String>) -> ValueQuery {
        let name = name.into();
        ValueQuery::new(
            crate::parser::Kind::Attribute,
            format!("{}::attr({name})", self.node.selector),
        )
        .with_trim(self.node.trim)
        .with_values(self.filter_map_base(|element| {
            element
                .value()
                .attr(&name)
                .map(|value| Value::String(trim_text(value, self.node.trim)))
        }))
    }

    fn extract(&self) -> Vec<String> {
        match projection(&self.node.selector) {
            Projection::Node(base) => self.map_select(base, |element| element.html()),
            Projection::Text(base) => self.text_for(base),
            Projection::Attribute(base, name) => self.attr_for(base, &name),
        }
    }

    fn text_for(&self, selector: &str) -> Vec<String> {
        self.map_select(selector, |element| trim_text(&node_text(&element), self.node.trim))
    }

    fn attr_for(&self, selector: &str, name: &str) -> Vec<String> {
        self.filter_map_select(selector, |element| {
            element.value().attr(name).map(|value| trim_text(value, self.node.trim))
        })
    }

    fn map_base<T>(&self, map: impl Fn(ElementRef<'_>) -> T) -> Vec<T> {
        match projection(&self.node.selector) {
            Projection::Node(base) | Projection::Text(base) | Projection::Attribute(base, _) => {
                self.map_select(base, map)
            }
        }
    }

    fn filter_map_base<T>(&self, map: impl Fn(ElementRef<'_>) -> Option<T>) -> Vec<T> {
        match projection(&self.node.selector) {
            Projection::Node(base) | Projection::Text(base) | Projection::Attribute(base, _) => {
                self.filter_map_select(base, map)
            }
        }
    }

    fn map_select<T>(&self, selector: &str, map: impl Fn(ElementRef<'_>) -> T) -> Vec<T> {
        let document = Html::parse_document(&self.input);
        let Ok(selector) = Selector::parse(selector) else {
            return Vec::new();
        };

        document.select(&selector).map(map).collect()
    }

    fn filter_map_select<T>(
        &self,
        selector: &str,
        map: impl Fn(ElementRef<'_>) -> Option<T>,
    ) -> Vec<T> {
        let document = Html::parse_document(&self.input);
        let Ok(selector) = Selector::parse(selector) else {
            return Vec::new();
        };

        document.select(&selector).filter_map(map).collect()
    }
}

enum Projection<'a> {
    Node(&'a str),
    Text(&'a str),
    Attribute(&'a str, String),
}

fn projection(selector: &str) -> Projection<'_> {
    if let Some(base) = selector.strip_suffix("::text") {
        return Projection::Text(base.trim());
    }

    if let Some(base) = selector.strip_suffix(')') {
        if let Some((base, attr)) = base.rsplit_once("::attr(") {
            return Projection::Attribute(base.trim(), attr.trim().to_string());
        }
    }

    Projection::Node(selector)
}

fn node_text(element: &ElementRef<'_>) -> String {
    element.text().collect::<Vec<_>>().join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_query_returns_node_backed_value_queries() {
        let query = CssQuery::new("<h1 class='title'>Hello</h1>", "h1.title");

        assert_eq!(query.node.selector, "h1.title");
        assert!(query.text().trim);
        assert!(!query.html().trim);
    }

    #[test]
    fn css_query_supports_text_projection() {
        let query = CssQuery::new("<h1 class='title'> Hello </h1>", "h1.title::text");

        assert_eq!(query.one().as_deref(), Some("Hello"));
    }

    #[test]
    fn css_query_supports_attr_projection() {
        let query = CssQuery::new(
            "<a class='link' href='/detail'>post</a>",
            "a.link::attr(href)",
        );

        assert_eq!(query.all(), vec!["/detail".to_string()]);
    }
}
