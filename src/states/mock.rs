//! G2: a project's mock server, as its dialog runs it: started on a port
//! of this machine's loopback address (`this_build::start_mock`, the
//! desktop's alone), answering from the project's examples as they are
//! now — what answers is made again shortly after the project changes —
//! and the requests it answered, newest first. It stops when the reader
//! stops it or the project closes.

use super::WorkspaceState;
use crate::this_build;
use cyberapi_core::mock::{Heard, Routes};
use futures::StreamExt as _;
use gpui_kit::{Context, Subscription, Task, WeakEntity};
use std::time::Duration;

/// The port a mock server is offered first.
pub const DEFAULT_PORT: u16 = 4010;

/// How many answered requests the log keeps.
const HEARD_KEPT: usize = 100;

/// How long the project is left to settle before what answers is made
/// again.
const REBUILD_DELAY: Duration = Duration::from_millis(300);

/// A running server, whichever build made it.
pub trait MockHandle: Send {
    /// Where it listens: `http://127.0.0.1:4010`.
    fn address(&self) -> String;
    fn set_routes(&self, routes: Routes);
}

pub struct MockState {
    workspace: WeakEntity<WorkspaceState>,
    port: u16,
    server: Option<Box<dyn MockHandle>>,
    starting: bool,
    /// Why the last start failed.
    error: Option<String>,
    heard: Vec<Heard>,
    /// How many APIs answer, and how many of them with an example.
    count: (usize, usize),
    _listening: Option<Task<()>>,
    _rebuild: Option<Task<()>>,
    _starting: Option<Task<()>>,
    _observe: Option<Subscription>,
}

impl MockState {
    pub fn new(workspace: WeakEntity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let count = workspace
            .upgrade()
            .map(|workspace| Routes::of(workspace.read(cx).tree_data()).count())
            .unwrap_or_default();
        let observe = workspace
            .upgrade()
            .map(|workspace| cx.observe(&workspace, |this, _, cx| this.rebuild_later(cx)));
        Self {
            workspace,
            port: DEFAULT_PORT,
            server: None,
            starting: false,
            error: None,
            heard: Vec::new(),
            count,
            _listening: None,
            _rebuild: None,
            _starting: None,
            _observe: observe,
        }
    }

    /// G2b: where the project's shared mock routes live on the bridge.
    pub fn server_url(&self, cx: &gpui_kit::App) -> Option<String> {
        self.workspace.upgrade()?.read(cx).server_mock_url(cx)
    }

    fn routes(&self, cx: &Context<Self>) -> Routes {
        self.workspace
            .upgrade()
            .map(|workspace| Routes::of(workspace.read(cx).tree_data()))
            .unwrap_or_default()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_port(&mut self, port: u16, cx: &mut Context<Self>) {
        if self.port != port {
            self.port = port;
            cx.notify();
        }
    }

    /// Where it listens, while it runs.
    pub fn address(&self) -> Option<String> {
        self.server.as_ref().map(|server| server.address())
    }

    pub fn starting(&self) -> bool {
        self.starting
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The requests answered, newest first.
    pub fn heard(&self) -> &[Heard] {
        &self.heard
    }

    /// How many APIs answer, and how many of them with an example.
    pub fn count(&self) -> (usize, usize) {
        self.count
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.server.is_some() || self.starting {
            return;
        }
        self.starting = true;
        self.error = None;
        let routes = self.routes(cx);
        self.count = routes.count();
        let (heard, mut told) = futures::channel::mpsc::unbounded();
        let starting = this_build::start_mock(self.port, routes, heard, cx);
        self._starting = Some(cx.spawn(async move |this, cx| {
            let started = starting.await;
            let _ = this.update(cx, |this, cx| {
                this.starting = false;
                match started {
                    Ok(server) => {
                        this.server = Some(server);
                        this.heard.clear();
                    }
                    Err(why) => this.error = Some(why),
                }
                cx.notify();
            });
        }));
        self._listening = Some(cx.spawn(async move |this, cx| {
            while let Some(heard) = told.next().await {
                let kept = this.update(cx, |this, cx| {
                    this.heard.insert(0, heard);
                    this.heard.truncate(HEARD_KEPT);
                    cx.notify();
                });
                if kept.is_err() {
                    return;
                }
            }
        }));
        cx.notify();
    }

    /// Stops it: the port is free again.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.server = None;
        self._listening = None;
        self._starting = None;
        self.starting = false;
        cx.notify();
    }

    /// The project changed: once it settles, what answers is made again.
    fn rebuild_later(&mut self, cx: &mut Context<Self>) {
        self._rebuild = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REBUILD_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let routes = this.routes(cx);
                this.count = routes.count();
                if let Some(server) = &this.server {
                    server.set_routes(routes);
                }
                cx.notify();
            });
        }));
    }
}
