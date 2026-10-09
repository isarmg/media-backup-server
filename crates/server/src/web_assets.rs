//! The Foundation-generated inventory and bytes are part of this executable.
include!(concat!(env!("OUT_DIR"), "/xcss-web-assets.rs"));

pub(crate) fn response(
    path: &str,
    method: &axum::http::Method,
    headers: &axum::http::HeaderMap,
) -> axum::response::Response {
    xcss_web_assets::response(ASSETS, path, method, headers).map(axum::body::Body::from)
}
