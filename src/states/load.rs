//! A7: a load test of one API (`cyberapi_service::load`): what it has sent
//! so far, and what that comes to (`cyberapi_core::load::summary`), taken
//! in a frame at a time. It is only shown: a new one, or closing it,
//! forgets it, and nothing is in the API's history.

use super::WorkspaceState;
use super::api::remember_typed;
use cyberapi_core::load::{Plan, Sample, Summary, summary};
use cyberapi_core::model::HttpRequest;
use cyberapi_core::send;
use cyberapi_core::template::{Answers, Prompt};
use cyberapi_core::{CollectionId, ItemId};
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Context, Entity, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::Arc;
use web_time::{Duration, Instant};

/// How often what has come in is counted again: a summary sorts every
/// send's time.
const FRAME: Duration = Duration::from_millis(300);

pub struct LoadState {
    workspace: WeakEntity<WorkspaceState>,
    collection: CollectionId,
    api: ItemId,
    samples: Vec<Sample>,
    summary: Summary,
    plan: Option<Plan>,
    running: Option<Task<()>>,
    started: Option<Instant>,
    took: Option<Duration>,
    /// When the last send so far ended, from the first's start: how long
    /// the sends took, the wait for the screen and the preparing left out
    /// (BUG-7).
    sent_for: Duration,
    /// Why nothing was sent.
    failure: Option<String>,
}

impl LoadState {
    pub fn new(workspace: Entity<WorkspaceState>, api: ItemId, cx: &mut Context<Self>) -> Self {
        let collection = workspace.read(cx).collection().id.clone();
        Self {
            workspace: workspace.downgrade(),
            collection,
            api,
            samples: Vec::new(),
            summary: Summary::default(),
            plan: None,
            running: None,
            started: None,
            took: None,
            sent_for: Duration::ZERO,
            failure: None,
        }
    }

    /// The API's request as its open tab or the project has it now (A7b).
    pub fn request(&self, cx: &App) -> Option<HttpRequest> {
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        if let Some(kept) = workspace.kept_api(&self.api) {
            return Some(kept.read(cx).request().clone());
        }
        workspace
            .tree_data()
            .apis
            .iter()
            .find(|api| api.id == self.api)
            .map(|api| api.request.clone())
    }

    /// D1: the `prompt`s it asks for, answered before it starts.
    pub fn prompts(&self, cx: &App) -> Vec<Prompt> {
        let (Some(workspace), Some(request)) = (self.workspace.upgrade(), self.request(cx)) else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        send::needs(&request, &workspace.global_headers(), &workspace.inherited(&self.api)).prompts
    }

    /// D6: whether starting is asked about first.
    pub fn asks_first(&self, cx: &App) -> bool {
        let (Some(workspace), Some(request)) = (self.workspace.upgrade(), self.request(cx)) else {
            return false;
        };
        workspace.read(cx).asks_before(&request, cx)
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }

    /// The plan of the test under way, or the last one.
    pub fn plan(&self) -> Option<Plan> {
        self.plan
    }

    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// How long the test has taken, or took.
    pub fn elapsed(&self) -> Option<Duration> {
        self.took.or_else(|| self.started.map(|started| started.elapsed()))
    }

    /// Starts a test of the API as it is now, with what was typed for its
    /// `prompt`s.
    pub fn start(&mut self, plan: Plan, prompts: HashMap<String, String>, cx: &mut Context<Self>) {
        let (Some(workspace), Some(request)) = (self.workspace.upgrade(), self.request(cx)) else {
            return;
        };
        let sender = {
            let workspace = workspace.read(cx);
            let mut sender = workspace.run_sender(cx);
            sender.inherited = workspace.inherited(&self.api);
            sender
        };
        remember_typed(&self.collection, &prompts, cx);
        let plan = plan.bounded();
        self.plan = Some(plan);
        self.samples.clear();
        self.summary = Summary::default();
        self.failure = None;
        self.took = None;
        self.sent_for = Duration::ZERO;
        let started = Instant::now();
        self.started = Some(started);
        let (told, mut telling) = mpsc::unbounded::<Sample>();
        let call = cyberapi_service::load(
            sender,
            self.collection.clone(),
            request,
            Answers { file: None, prompts },
            plan,
            Arc::new(move |sample| {
                let _ = told.unbounded_send(sample);
            }),
        );
        let work = cx.background_spawn(call);
        self.running = Some(cx.spawn(async move |this, cx| {
            let frames = async {
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
            };
            // The test's end is the channel's: every sample is in by then.
            let (failure, ()) = futures::join!(work, frames);
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                // Over when the last send ended: the frames' waits after it
                // are not the test's.
                this.took = Some(if this.samples.is_empty() {
                    started.elapsed()
                } else {
                    this.sent_for
                });
                this.failure = failure.err();
                this.summary = summary(&this.samples, this.sent_for);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Stops the test: the sends under way are dropped, what came is kept.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.running.take().is_some() {
            if let Some(started) = self.started {
                self.took = Some(started.elapsed());
            }
            self.summary = summary(&self.samples, self.took.unwrap_or_default());
            cx.notify();
        }
    }

    fn told(&mut self, told: Vec<Sample>, cx: &mut Context<Self>) {
        if let Some(last) = told.iter().map(|sample| sample.at).max() {
            self.sent_for = self.sent_for.max(last);
        }
        self.samples.extend(told);
        self.summary = summary(&self.samples, self.sent_for);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use crate::states::AppStore;
    use cyberapi_core::load::Limit;
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{Exchange, HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::TestAppContext;

    /// Every request answered `200 ok`, at once.
    struct Answers;

    impl HttpEngine for Answers {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(async {
                Ok(Exchange {
                    status: 200,
                    body: b"ok".to_vec(),
                    ..Default::default()
                })
            })
        }
    }

    /// BUG-7: the test took until its last send ended — not until the
    /// screen was next told, nor from before the request was prepared —
    /// and so many a second.
    #[gpui_kit::test]
    async fn a_test_takes_as_long_as_its_sends(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                storage.clone(),
                Arc::new(Answers),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let workspace = cx.new(|cx| {
            let sender = AppStore::global(cx).sender(cx);
            WorkspaceState::new(sender, collection, cx)
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| workspace.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        let id = workspace.read_with(cx, |workspace, _| workspace.tabs()[0].id.clone());
        let api = workspace.read_with(cx, |workspace, _| workspace.kept_api(&id).cloned().expect("kept"));
        api.update(cx, |api, cx| api.set_uri("http://a.test/".into(), cx));
        cx.run_until_parked();

        let load = cx.new(|cx| LoadState::new(workspace.clone(), id, cx));
        let plan = Plan {
            senders: 2,
            limit: Limit::Count(20),
        };
        load.update(cx, |load, cx| load.start(plan, HashMap::new(), cx));
        for _ in 0..50 {
            cx.run_until_parked();
            if !load.read_with(cx, |load, _| load.running()) {
                break;
            }
            cx.executor().advance_clock(FRAME);
        }
        load.read_with(cx, |load, _| {
            assert!(!load.running());
            assert_eq!(load.summary().sent, 20);
            let last = load.samples.iter().map(|sample| sample.at).max().expect("sent");
            assert_eq!(load.elapsed(), Some(last));
            let per_second = 20.0 / last.as_secs_f64();
            assert!((load.summary().per_second - per_second).abs() < 1e-6 * per_second);
        });
    }
}
