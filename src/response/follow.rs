use crate::request::{CallbackTarget, Metadata, Request};
use crate::response::Response;
use crate::value::Value;
use std::collections::BTreeMap;

pub fn build_follow_request(
    response: &Response,
    url: String,
    callback: Option<String>,
    meta_patch: &Metadata,
) -> Request {
    let mut request = match response.request.as_deref() {
        Some(parent) => Request::new(url).with_mode(parent.mode),
        None => Request::new(url),
    };

    request.meta = merge_meta(&response.meta, meta_patch);
    request.callback = callback.map(CallbackTarget::new);
    request
}

pub fn merge_meta(current: &Metadata, patch: &BTreeMap<String, Value>) -> Metadata {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    merged
}
