use crate::middleware::traits::Middleware;

#[derive(Default)]
pub struct CookiesMiddleware;

impl Middleware for CookiesMiddleware {}
