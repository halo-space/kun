use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct DedupMiddleware;

impl Middleware for DedupMiddleware {}
