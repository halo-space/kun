pub mod context;
pub mod types;

use crate::error::SpiderError;
use crate::response::Response;

#[derive(Default)]
pub struct Engine;

impl Engine {
    pub fn execute_once(&self) -> Result<Option<Response>, SpiderError> {
        Ok(None)
    }
}
