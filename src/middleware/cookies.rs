use crate::middleware::traits::Middleware;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct CookiesMiddleware;

impl CookiesMiddleware {
    pub fn new(_options: &BTreeMap<String, Value>) -> Self {
        Self
    }
}

impl Middleware for CookiesMiddleware {}
