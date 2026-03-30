use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub query: BTreeMap<String, String>,
    pub cookies: BTreeMap<String, String>,
    pub allow_redirects: bool,
}

impl Config {
    pub fn with_query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.insert(key.into(), value.into());
        self
    }

    pub fn with_cookie(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.cookies.insert(key.into(), value.into());
        self
    }

    pub fn with_redirects(mut self, allow_redirects: bool) -> Self {
        self.allow_redirects = allow_redirects;
        self
    }
}
