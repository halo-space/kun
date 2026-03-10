use crate::parser::query::NodeQuery;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XPathQuery {
    pub node: NodeQuery,
}

impl XPathQuery {
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
