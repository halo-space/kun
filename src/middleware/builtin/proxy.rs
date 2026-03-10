use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct ProxyMiddleware;

impl Middleware for ProxyMiddleware {}
