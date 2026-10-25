//! The page's way to the bridge: GPUI's HTTP client, which in a page is the
//! browser's `fetch`. Same origin, so the login cookie rides along without
//! asking.

use cyberapi_store_bridge::{Response, Streamed, Transport};
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

/// How much of an answer's body one read takes.
const PIECE: usize = 16 * 1024;

impl FetchTransport {
    /// POSTs `body` to `path`: the answer, its body still to be read.
    async fn posted(
        client: Arc<dyn HttpClient>,
        url: String,
        body: Vec<u8>,
    ) -> Result<gpui_kit::http_client::Response<AsyncBody>, String> {
        let request = Request::builder()
            .method("POST")
            .uri(&url)
            .header("content-type", "application/json")
            .body(AsyncBody::from(body))
            .map_err(|e| format!("the request could not be built: {e}"))?;
        let response = client
            .send(request)
            .await
            .map_err(|e| format!("{url} is unreachable: {e}"))?;
        if response.status().as_u16() == 401 {
            sign_in_again();
        }
        Ok(response)
    }
}

impl Transport for FetchTransport {
    /// The body read as it comes, from the browser's stream: dropping the
    /// pieces cancels it, and the bridge sees the page leave.
    fn post_stream(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Streamed, String>> + Send>> {
        let (client, url) = (self.client.clone(), format!("{}{path}", self.origin));
        Box::pin(async move {
            let response = Self::posted(client, url, body).await?;
            let status = response.status().as_u16();
            let pieces = futures::stream::unfold(Some(response.into_body()), |body| async move {
                let mut body = body?;
                let mut piece = vec![0; PIECE];
                match body.read(&mut piece).await {
                    Ok(0) => None,
                    Ok(read) => {
                        piece.truncate(read);
                        Some((Ok(piece), Some(body)))
                    }
                    Err(e) => Some((Err(format!("the bridge's answer broke off: {e}")), None)),
                }
            });
            Ok(Streamed {
                status,
                pieces: Box::pin(pieces),
            })
        })
    }

    fn post(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
        let (client, url) = (self.client.clone(), format!("{}{path}", self.origin));
        Box::pin(async move {
            let mut response = Self::posted(client, url, body).await?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            response
                .body_mut()
                .read_to_end(&mut body)
                .await
                .map_err(|e| format!("the bridge's answer could not be read: {e}"))?;
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
