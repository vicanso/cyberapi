//! A3: a run — the APIs of a folder or a project sent one after another,
//! once or once for each row of a data file, each pass's values (the row's,
//! and what each API took from its answer, A2) read by the requests after
//! them (`cyberapi_core::run`). Every send is kept in the history as any
//! send is; how each went is told as it ends, to be shown.

use crate::send::{Sender, send_with};
use cyberapi_core::assertions::{self, Outcomes};
use cyberapi_core::inherit::Inherited;
use cyberapi_core::model::HttpRequest;
use cyberapi_core::response::HttpResponse;
use cyberapi_core::run::{self as rules, RunOptions, Values};
use cyberapi_core::template::Answers;
use cyberapi_core::{CollectionId, ItemId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// One API of a run, as the caller holds it now: its request, and what it
/// takes from its folders and its project (#66).
#[derive(Clone)]
pub struct RunStep {
    pub api: ItemId,
    pub request: HttpRequest,
    pub inherited: Inherited,
}

/// How one send of a run went.
#[derive(Clone, Debug)]
pub struct StepResult {
    /// Which pass: the data file's row, from 0.
    pub pass: usize,
    /// Which of the run's steps.
    pub step: usize,
    /// A9: how many attempts it took (1 when no retry was needed).
    pub attempts: u32,
    /// What came back, as a list shows it (`HttpResponse::summary`); why
    /// nothing went when it did not.
    pub response: Result<HttpResponse, String>,
    /// The API's enabled assertions, checked.
    pub outcomes: Outcomes,
    /// What it took for the requests after it (A2), by name.
    pub extracted: Values,
}

impl StepResult {
    /// It was answered, and every assertion held.
    pub fn held(&self) -> bool {
        match &self.response {
            Ok(response) => rules::held(response, &self.outcomes),
            Err(_) => false,
        }
    }
}

/// Sends `steps` once for each of `rows`, or once when there are none,
/// telling `report` how each went. `answers` are the reader's to the run's
/// `prompt`s, asked for beforehand. Dropping the future stops the run.
pub async fn run(
    sender: Sender,
    collection: CollectionId,
    steps: Vec<RunStep>,
    rows: Vec<Values>,
    answers: Answers,
    report: Arc<dyn Fn(StepResult) + Send + Sync>,
) {
    run_with_options(
        sender,
        collection,
        steps,
        rows,
        answers,
        RunOptions::default(),
        Arc::new(|_| Box::pin(async {})),
        report,
    )
    .await;
}

/// Async sleep callback used by [`run_with_options`] so both native and
/// browser executors can provide their timer.
pub type SleepFn = Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A9: sends `steps` with `options` (pause between sends, retry on failure,
/// stop on first failure).
#[allow(clippy::too_many_arguments)]
pub async fn run_with_options(
    sender: Sender,
    collection: CollectionId,
    steps: Vec<RunStep>,
    rows: Vec<Values>,
    answers: Answers,
    options: RunOptions,
    sleep: SleepFn,
    report: Arc<dyn Fn(StepResult) + Send + Sync>,
) {
    let options = options.bounded();
    let delay = options.delay();
    let max_attempts = 1 + options.retries;
    let passes = if rows.is_empty() { vec![Values::new()] } else { rows };
    let mut first_send = true;
    'run: for (pass, row) in passes.into_iter().enumerate() {
        // Each pass starts from its row: what one took is its own.
        let mut values = row;
        for (index, step) in steps.iter().enumerate() {
            if !first_send && !delay.is_zero() {
                sleep(delay).await;
            }
            first_send = false;
            let mut attempt = 0;
            let result = loop {
                attempt += 1;
                if attempt > 1 && !delay.is_zero() {
                    sleep(delay).await;
                }
                let step_sender = Sender {
                    inherited: step.inherited.clone(),
                    ..sender.clone()
                };
                let sent = send_with(
                    step_sender,
                    collection.clone(),
                    step.api.clone(),
                    step.request.clone(),
                    answers.clone(),
                    None,
                    &values,
                )
                .await;
                let (response, outcomes, extracted) = match sent {
                    Ok(response) => {
                        let json = assertions::reads_json(&step.request.assertions)
                            .then(|| response.body_json())
                            .flatten();
                        let outcomes = assertions::check_all(&step.request.assertions, &response, json.as_ref());
                        let extracted = assertions::extract_all(&step.request.extractions, &response);
                        (Ok(response.summary()), outcomes, extracted)
                    }
                    Err(failure) => (Err(failure.to_string()), Vec::new(), Vec::new()),
                };
                let result = StepResult {
                    pass,
                    step: index,
                    attempts: attempt,
                    response,
                    outcomes,
                    extracted,
                };
                if result.held() || attempt >= max_attempts {
                    break result;
                }
            };
            let step_held = result.held();
            for (name, value) in &result.extracted {
                rules::set(&mut values, name, value.clone());
            }
            report(result);
            if !step_held && options.bail {
                break 'run;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{ContentType, HttpMethod};
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::response::{Exchange, HttpStats};
    use cyberapi_core::send::PreparedRequest;
    use cyberapi_core::template::{NoEnv, NoFiles};
    use cyberapi_http::{HttpEngine, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, Storage};
    use std::sync::Mutex;

    struct SequenceEngine {
        statuses: Mutex<Vec<u16>>,
    }

    impl HttpEngine for SequenceEngine {
        fn send(&self, _request: PreparedRequest) -> SendFuture {
            let status = self.statuses.lock().expect("statuses").remove(0);
            Box::pin(async move {
                Ok(Exchange {
                    status,
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
    fn run_retries_and_bails_when_configured() {
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
            // Step 0 fails first (500), succeeds on retry (200).
            // Step 1 fails twice (500, 500) -> exhausts 1 retry, then bail stops step 2.
            let engine = Arc::new(SequenceEngine {
                statuses: Mutex::new(vec![500, 200, 500, 500, 200]),
            });
            let sender = Sender {
                storage,
                engine,
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
            let step = |id: &str| RunStep {
                api: ItemId::new(id),
                request: HttpRequest {
                    method: HttpMethod::Get,
                    uri: format!("https://api.example.com/{id}"),
                    content_type: ContentType::Json,
                    ..Default::default()
                },
                inherited: Default::default(),
            };
            let steps = vec![step("a"), step("b"), step("c")];
            let sleeps = Arc::new(Mutex::new(Vec::new()));
            let sleeps_recorded = sleeps.clone();
            let results = Arc::new(Mutex::new(Vec::new()));
            let results_recorded = results.clone();
            run_with_options(
                sender,
                collection.id,
                steps,
                Vec::new(),
                Answers::default(),
                RunOptions {
                    delay_ms: 25,
                    retries: 1,
                    bail: true,
                },
                Arc::new(move |d| {
                    sleeps_recorded.lock().expect("sleeps").push(d);
                    Box::pin(async {})
                }),
                Arc::new(move |r| {
                    results_recorded.lock().expect("results").push(r);
                }),
            )
            .await;

            let results = results.lock().expect("results");
            assert_eq!(results.len(), 2, "step c was skipped due to bail");
            assert!(results[0].held());
            assert_eq!(results[0].attempts, 2);
            assert!(!results[1].held());
            assert_eq!(results[1].attempts, 2);
            // Sleeps: retry of step 0, before step 1, retry of step 1 = 3 sleeps of 25ms
            assert_eq!(sleeps.lock().expect("sleeps").len(), 3);
        });
    }
}
