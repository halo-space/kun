use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct RetryByErrorMiddleware;

impl Middleware for RetryByErrorMiddleware {}
