//! A7 / A7b: a load test of one API (`cyberapi_core::load`): its variables,
//! the jar's cookies and its token are gathered once, then sent over and over
//! by the plan's senders at once, each send told as it ends. Dynamic template
//! functions (`uuid`, `random`, `randomString`, `now`, `timestamp`) are
//! evaluated afresh for each send when present. What is measured is the one
//! request: a redirect is not followed and a Digest challenge not answered.
//! Nothing goes into the history, and no cookie the answers set is kept.
//! Dropping the future stops it.

use crate::oauth;
use crate::send::{SendFailure, Sender, add_cookies, prepare_load};
use cyberapi_core::grpc::{GrpcBody, GrpcStatus};
use cyberapi_core::load::{Plan, Sample};
use cyberapi_core::model::{ContentType, HttpMethod, HttpRequest};
use cyberapi_core::template::Answers;
use cyberapi_core::{CollectionId, graphql_ws};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use web_time::Instant;

/// Runs `plan` on `request`, the API's as the editor holds it, telling
/// `report` of each send. `answers` are the reader's to its `prompt`s.
/// `Err` says why nothing was sent.
pub async fn load(
    sender: Sender,
    collection: CollectionId,
    request: HttpRequest,
    answers: Answers,
    plan: Plan,
    report: Arc<dyn Fn(Sample) + Send + Sync>,
) -> Result<(), String> {
    let refused = match request.method {
        HttpMethod::WebSocket => Some(SendFailure::WebSocket),
        HttpMethod::Grpc if GrpcBody::read(&request.body).client_streams() => Some(SendFailure::GrpcStream),
        _ => None,
    };
    if let Some(refused) = refused {
        return Err(refused.to_string());
    }
    let sender = sender.for_request(&request);
    let (mut prepared, template) = prepare_load(&sender, &collection, &request, &answers)
        .await
        .map_err(|failure| failure.to_string())?;
    if request.content_type == ContentType::GraphQl && graphql_ws::is_socket_url(&prepared.url) {
        return Err(SendFailure::GraphQlSocket.to_string());
    }
    let had_cookie = prepared.header("cookie").is_some();
    if !had_cookie {
        add_cookies(&sender, &mut prepared)
            .await
            .map_err(|failure| failure.to_string())?;
    }
    let jar_cookie = (!had_cookie)
        .then(|| prepared.header("cookie").map(str::to_string))
        .flatten();
    let signed = oauth::sign(&sender, &mut prepared, false).await?;
    let oauth_header = (signed != oauth::Signed::No)
        .then(|| prepared.header("authorization").map(str::to_string))
        .flatten();
    let grpc = request.method == HttpMethod::Grpc;
    let plan = plan.bounded();
    let started = AtomicU32::new(0);
    let begun = Instant::now();
    let one = || async {
        loop {
            let turn = started.fetch_add(1, Ordering::Relaxed);
            if !plan.goes_on(turn, begun.elapsed()) {
                break;
            }
            let call_prepared = if turn > 0 && template.dynamic {
                match template.prepare_again(&sender, &request, turn) {
                    Ok(mut next) => {
                        if let Some(cookie) = &jar_cookie
                            && next.header("cookie").is_none()
                        {
                            next.headers.push(("Cookie".into(), cookie.clone()));
                        }
                        if let Some(auth) = &oauth_header {
                            next.headers
                                .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
                            next.headers.push(("Authorization".into(), auth.clone()));
                        }
                        next
                    }
                    Err(_) => prepared.clone(),
                }
            } else {
                prepared.clone()
            };
            let at = Instant::now();
            let answered = sender.engine.send(call_prepared).await;
            let took = at.elapsed();
            report(match answered {
                Ok(exchange) => Sample {
                    at: begun.elapsed(),
                    took,
                    status: Some(exchange.status),
                    grpc: grpc.then(|| grpc_code(&exchange)),
                    error: None,
                    size: exchange.body_size,
                },
                Err(error) => Sample {
                    at: begun.elapsed(),
                    took,
                    status: None,
                    grpc: None,
                    error: Some(error.to_string()),
                    size: 0,
                },
            });
        }
    };
    futures::future::join_all((0..plan.senders_needed()).map(|_| one())).await;
    Ok(())
}

/// A gRPC call's status (BUG-8): in the trailers, or the headers of an
/// answer that is trailers only. None at all is UNKNOWN, as clients take
/// it.
fn grpc_code(exchange: &cyberapi_http::Exchange) -> u32 {
    let ended: Vec<(String, String)> = exchange.headers.iter().chain(&exchange.trailers).cloned().collect();
    GrpcStatus::read(&ended).map_or(2, |status| status.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::load::Limit;
    use cyberapi_core::model::{ContentType, HttpMethod, KvParam};
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::response::{Exchange, HttpStats};
    use cyberapi_core::send::PreparedRequest;
    use cyberapi_core::template::{NoEnv, NoFiles};
    use cyberapi_http::{HttpEngine, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, Storage};
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingEngine {
        sent: Mutex<Vec<PreparedRequest>>,
    }

    impl HttpEngine for RecordingEngine {
        fn send(&self, request: PreparedRequest) -> SendFuture {
            self.sent.lock().expect("sent").push(request);
            Box::pin(async {
                Ok(Exchange {
                    status: 200,
                    headers: Vec::new(),
                    body: b"{}".to_vec(),
                    body_size: 2,
                    truncated: false,
                    stats: HttpStats::default(),
                    trailers: Vec::new(),
                    stream: None,
                })
            })
        }
    }

    #[test]
    fn load_evaluates_dynamic_templates_for_every_send() {
        futures::executor::block_on(async {
            let store = Arc::new(MemoryStore::new());
            let storage = Storage::from_backend(store);
            let collection = storage
                .collections()
                .create(NewCollection {
                    name: "Demo".into(),
                    description: String::new(),
                    owner: None,
                    id: None,
                })
                .await
                .expect("create collection");
            let engine = Arc::new(RecordingEngine::default());
            let sender = Sender {
                storage,
                engine: engine.clone(),
                files: Arc::new(NoFiles("no files")),
                env: Arc::new(NoEnv("no env")),
                user_agent: "CyberAPI/test".into(),
                timeouts: Timeouts::default(),
                sending: SendPrefs::default(),
                inherited: Default::default(),
                environment: None,
                tokens: Default::default(),
                secrets: Default::default(),
            };
            let request = HttpRequest {
                method: HttpMethod::Post,
                uri: "https://api.example.com/orders".into(),
                content_type: ContentType::Json,
                headers: vec![KvParam {
                    key: "X-Request-Id".into(),
                    value: "{{uuid()}}".into(),
                    enabled: true,
                    ..Default::default()
                }],
                body: r#"{"nonce":"{{randomString(12)}}"}"#.into(),
                ..Default::default()
            };
            let plan = Plan {
                senders: 2,
                limit: Limit::Count(5),
            };
            load(
                sender,
                collection.id,
                request,
                Answers::default(),
                plan,
                Arc::new(|_| {}),
            )
            .await
            .expect("load succeeds");

            let sent = engine.sent.lock().expect("sent");
            assert_eq!(sent.len(), 5);
            let request_ids: HashSet<String> = sent
                .iter()
                .filter_map(|req| req.header("x-request-id").map(str::to_string))
                .collect();
            assert_eq!(request_ids.len(), 5, "each send drew its own UUID");
            let bodies: HashSet<Vec<u8>> = sent.iter().map(|req| req.body.clone()).collect();
            assert_eq!(bodies.len(), 5, "each send drew its own randomString");
        });
    }
}
