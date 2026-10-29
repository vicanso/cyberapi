//! A7: a load test of one API (`cyberapi_core::load`): prepared once as a
//! send prepares it — its variables, the jar's cookies, its token — then
//! sent over and over by the plan's senders at once, each send told as it
//! ends. What is measured is the one request: a redirect is not followed,
//! a Digest challenge not answered, and the template functions are run
//! once, for all the sends. Nothing goes into the history, and no cookie
//! the answers set is kept. Dropping the future stops it.

use crate::oauth;
use crate::send::{SendFailure, Sender, Unanswered, add_cookies, prepare};
use cyberapi_core::grpc::GrpcBody;
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
    let (mut prepared, _) = prepare(&sender, &collection, &request, &answers, Unanswered::Stops, None, &[])
        .await
        .map_err(|failure| failure.to_string())?;
    if request.content_type == ContentType::GraphQl && graphql_ws::is_socket_url(&prepared.url) {
        return Err(SendFailure::GraphQlSocket.to_string());
    }
    if prepared.header("cookie").is_none() {
        add_cookies(&sender, &mut prepared)
            .await
            .map_err(|failure| failure.to_string())?;
    }
    oauth::sign(&sender, &mut prepared, false).await?;
    let plan = plan.bounded();
    let started = AtomicU32::new(0);
    let begun = Instant::now();
    let one = || async {
        while plan.goes_on(started.fetch_add(1, Ordering::Relaxed), begun.elapsed()) {
            let at = Instant::now();
            let answered = sender.engine.send(prepared.clone()).await;
            let took = at.elapsed();
            report(match answered {
                Ok(exchange) => Sample {
                    at: begun.elapsed(),
                    took,
                    status: Some(exchange.status),
                    error: None,
                    size: exchange.body_size,
                },
                Err(error) => Sample {
                    at: begun.elapsed(),
                    took,
                    status: None,
                    error: Some(error.to_string()),
                    size: 0,
                },
            });
        }
    };
    futures::future::join_all((0..plan.senders_needed()).map(|_| one())).await;
    Ok(())
}
