//! The page's way to the bridge: GPUI's HTTP client, which in a page is the
//! browser's `fetch`. Same origin, so the login cookie rides along without
//! asking.

use cyberapi_store_bridge::{Response, Transport};
use futures::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, HttpClient, http::Request};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub struct FetchTransport {
    client: Arc<dyn HttpClient>,
    /// The bridge's address: the page's own, without a trailing slash.
    origin: String,
}

impl FetchTransport {
    pub fn new(client: Arc<dyn HttpClient>, origin: String) -> Self {
        Self { client, origin }
    }
}

impl Transport for FetchTransport {
    fn post(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
        let (client, url) = (self.client.clone(), format!("{}{path}", self.origin));
        Box::pin(async move {
            let request = Request::builder()
                .method("POST")
                .uri(&url)
                .header("content-type", "application/json")
                .body(AsyncBody::from(body))
                .map_err(|e| format!("the request could not be built: {e}"))?;
            let mut response = client
                .send(request)
                .await
                .map_err(|e| format!("{url} is unreachable: {e}"))?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            response
                .body_mut()
                .read_to_end(&mut body)
                .await
                .map_err(|e| format!("the bridge's answer could not be read: {e}"))?;
            if status == 401 {
                sign_in_again();
            }
            Ok(Response { status, body })
        })
    }
}

/// The login lapsed, was ended elsewhere, or was ended here: reloading
/// brings the page's sign-in form back, which is the only place a login is
/// made.
pub fn sign_in_again() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}
