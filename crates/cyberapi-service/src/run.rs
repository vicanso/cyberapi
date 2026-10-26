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
use cyberapi_core::run::{self as rules, Values};
use cyberapi_core::template::Answers;
use cyberapi_core::{CollectionId, ItemId};
use std::sync::Arc;

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
    let passes = if rows.is_empty() { vec![Values::new()] } else { rows };
    for (pass, row) in passes.into_iter().enumerate() {
        // Each pass starts from its row: what one took is its own.
        let mut values = row;
        for (index, step) in steps.iter().enumerate() {
            let sender = Sender {
                inherited: step.inherited.clone(),
                ..sender.clone()
            };
            let sent = send_with(
                sender,
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
            for (name, value) in &extracted {
                rules::set(&mut values, name, value.clone());
            }
            report(StepResult {
                pass,
                step: index,
                response,
                outcomes,
                extracted,
            });
        }
    }
}
