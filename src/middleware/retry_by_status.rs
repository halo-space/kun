use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct RetryByStatusMiddleware;

impl Middleware for RetryByStatusMiddleware {}
