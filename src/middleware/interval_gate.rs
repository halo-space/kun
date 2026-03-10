use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct IntervalGateMiddleware;

impl Middleware for IntervalGateMiddleware {}
