//! A3: a run of a folder's APIs or the project's (`cyberapi_service::run`):
//! what it sends, in the tree's order, and how each send went as it comes.
//! The run is only shown: a new one, or closing it, forgets it; what it
//! sent is in each API's history.

use super::WorkspaceState;
use super::api::remember_typed;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::run::{self as rules, DataError};
use cyberapi_core::send;
use cyberapi_core::template::{Answers, Prompt};
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_service::{RunStep, StepResult};
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Context, Entity, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::Arc;
use web_time::{Duration, Instant};

/// What a frame takes in of a run's news at once.
const FRAME: Duration = Duration::from_millis(50);

/// An API a run sends, as its list shows it.
#[derive(Clone)]
pub struct Planned {
    pub api: ItemId,
    pub name: String,
    pub method: HttpMethod,
    pub uri: String,
}

pub struct RunState {
    workspace: WeakEntity<WorkspaceState>,
    collection: CollectionId,
    /// The folder it runs; `None` is the whole project.
    folder: Option<ItemId>,
    planned: Vec<Planned>,
    /// How many passes the run makes: the data file's rows, or one.
    passes: usize,
    results: Vec<StepResult>,
    running: Option<Task<()>>,
    started: Option<Instant>,
    took: Option<Duration>,
}

impl RunState {
    pub fn new(workspace: Entity<WorkspaceState>, folder: Option<ItemId>, cx: &mut Context<Self>) -> Self {
        let (collection, planned) = {
            let workspace = workspace.read(cx);
            let data = workspace.tree_data();
            let planned = rules::plan(data, folder.as_ref())
                .into_iter()
                .filter_map(|id| data.apis.iter().find(|api| api.id == id))
                .map(|api| Planned {
                    api: api.id.clone(),
                    name: api.name.clone(),
                    method: api.request.method,
                    uri: api.request.uri.clone(),
                })
                .collect();
            (workspace.collection().id.clone(), planned)
        };
        Self {
            workspace: workspace.downgrade(),
            collection,
            folder,
            planned,
            passes: 1,
            results: Vec::new(),
            running: None,
            started: None,
            took: None,
        }
    }

    /// The folder it runs; `None` is the whole project.
    pub fn folder(&self) -> Option<&ItemId> {
        self.folder.as_ref()
    }

    pub fn planned(&self) -> &[Planned] {
        &self.planned
    }

    pub fn passes(&self) -> usize {
        self.passes
    }

    pub fn results(&self) -> &[StepResult] {
        &self.results
    }

    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// How long the run has taken, or took.
    pub fn elapsed(&self) -> Option<Duration> {
        self.took.or_else(|| self.started.map(|started| started.elapsed()))
    }

    /// How many sends held, and how many did not.
    pub fn tally(&self) -> (usize, usize) {
        let held = self.results.iter().filter(|result| result.held()).count();
        (held, self.results.len() - held)
    }

    /// D1: the `prompt`s of the run's requests, each label once, for the
    /// reader to answer before it starts.
    pub fn prompts(&self, cx: &App) -> Vec<Prompt> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        let headers = workspace.global_headers();
        let mut prompts: Vec<Prompt> = Vec::new();
        for planned in &self.planned {
            let Some(api) = workspace.tree_data().apis.iter().find(|api| api.id == planned.api) else {
                continue;
            };
            let needs = send::needs(&api.request, &headers, &workspace.inherited(&api.id));
            for prompt in needs.prompts {
                if !prompts.iter().any(|known| known.label == prompt.label) {
                    prompts.push(prompt);
                }
            }
        }
        prompts
    }

    /// Starts the run over again: the APIs as they are now, once for each
    /// row of `data` (CSV or a JSON array; blank is once), with what was
    /// typed for its `prompt`s. A data file that does not read starts
    /// nothing and says why.
    pub fn start(
        &mut self,
        data: &str,
        prompts: HashMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Result<(), DataError> {
        let rows = rules::rows(data)?;
        let Some(workspace) = self.workspace.upgrade() else {
            return Ok(());
        };
        let (sender, steps) = {
            let workspace = workspace.read(cx);
            let data = workspace.tree_data();
            let steps: Vec<RunStep> = rules::plan(data, self.folder.as_ref())
                .into_iter()
                .filter_map(|id| data.apis.iter().find(|api| api.id == id))
                .map(|api| RunStep {
                    api: api.id.clone(),
                    request: api.request.clone(),
                    inherited: workspace.inherited(&api.id),
                })
                .collect();
            (workspace.run_sender(cx), steps)
        };
        remember_typed(&self.collection, &prompts, cx);
        self.passes = rows.len().max(1);
        self.results.clear();
        self.took = None;
        let started = Instant::now();
        self.started = Some(started);
        let (told, mut telling) = mpsc::unbounded::<StepResult>();
        let call = cyberapi_service::run(
            sender,
            self.collection.clone(),
            steps,
            rows,
            Answers { file: None, prompts },
            Arc::new(move |result| {
                let _ = told.unbounded_send(result);
            }),
        );
        let work = cx.background_spawn(call);
        self.running = Some(cx.spawn(async move |this, cx| {
            // Dropped with this task, the run stops.
            let _work = work;
            while let Some(first) = telling.next().await {
                let mut told = vec![first];
                while let Ok(more) = telling.try_recv() {
                    told.push(more);
                }
                if this.update(cx, |this, cx| this.told(told, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(FRAME).await;
            }
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                this.took = Some(started.elapsed());
                this.ended(cx);
                cx.notify();
            });
        }));
        cx.notify();
        Ok(())
    }

    fn told(&mut self, told: Vec<StepResult>, cx: &mut Context<Self>) {
        self.results.extend(told);
        cx.notify();
    }

    /// Stops the run where it is: what was sent stays.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.running.take().is_some() {
            self.took = self.started.map(|started| started.elapsed());
            self.ended(cx);
            cx.notify();
        }
    }

    /// What the run sent is in the histories: the API open shows it.
    fn ended(&self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let sent: Vec<ItemId> = self
            .results
            .iter()
            .filter_map(|result| self.planned.get(result.step).map(|planned| planned.api.clone()))
            .collect();
        workspace.update(cx, |workspace, cx| workspace.ran(&sent, cx));
    }
}
