use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct RateLimitMiddleware;

impl Middleware for RateLimitMiddleware {}
