use crate::request::{CallbackTarget, Metadata, Request};
use crate::response::Response;
use crate::value::Value;
use std::collections::BTreeMap;

pub fn build_follow_request(
    response: &Response,
    url: String,
    callback: Option<String>,
    meta: &Metadata,
) -> Request {
    let absolute_url = resolve_url(&response.url, &url);

    let mut request = match response.request.as_deref() {
        Some(parent) => Request::from_parent_for_follow(parent, absolute_url),
        None => Request::new(absolute_url),
    };

    request.meta = merge_meta(&response.meta, meta);
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

fn resolve_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }

    let base_url = match url::Url::parse(base) {
        Ok(u) => u,
        Err(_) => return url.to_string(),
    };

    match base_url.join(url) {
        Ok(u) => u.to_string(),
        Err(_) => url.to_string(),
    }
}
