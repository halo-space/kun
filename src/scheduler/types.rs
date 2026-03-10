use crate::request::Request;

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub request: Request,
}
