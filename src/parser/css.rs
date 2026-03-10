use crate::parser::query::NodeQuery;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CssQuery {
    pub node: NodeQuery,
}

impl CssQuery {
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            node: NodeQuery::new(selector),
        }
    }

    pub fn one(&self) -> Option<String> {
        self.node.one()
    }

    pub fn all(&self) -> Vec<String> {
        self.node.all()
    }

    pub fn text(&self) -> crate::parser::ValueQuery {
        self.node.text()
    }

    pub fn html(&self) -> crate::parser::ValueQuery {
        self.node.html()
    }

    pub fn attr(&self, name: impl Into<String>) -> crate::parser::ValueQuery {
        self.node.attr(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_query_returns_node_backed_value_queries() {
        let query = CssQuery::new("h1.title");

        assert_eq!(query.node.selector, "h1.title");
        assert!(query.text().trim);
        assert!(!query.html().trim);
    }
}
