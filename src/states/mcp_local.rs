//! H2: the desktop's local loopback MCP server (`127.0.0.1:<port>/v1/mcp`),
//! allowing local AI assistants (Claude Code, Cursor, etc.) to inspect, send
//! and edit projects on this machine without deploying `cyberapi-bridge`.

use crate::mcp_local::{self, McpFile};
use crate::this_build;
use cyberapi_http::HttpEngine;
use cyberapi_service::mcp::generate_local_token;
use cyberapi_store::Storage;
use gpui_kit::{Context, Task};
use std::sync::Arc;

/// A running local MCP server, whichever build made it.
pub trait LocalMcpHandle: Send {
    /// Its MCP endpoint: `http://127.0.0.1:4020/v1/mcp`.
    fn endpoint(&self) -> String;
    /// Updates the bearer token and permissions in place.
    fn set_access(&self, token: String, can_send: bool, can_write: bool);
}

pub struct LocalMcpState {
    storage: Storage,
    engine: Arc<dyn HttpEngine>,
    config: McpFile,
    server: Option<Box<dyn LocalMcpHandle>>,
    starting: bool,
    error: Option<String>,
    _starting: Option<Task<()>>,
}

impl LocalMcpState {
    pub fn new(storage: Storage, engine: Arc<dyn HttpEngine>, cx: &mut Context<Self>) -> Self {
        let config = mcp_local::read();
        let mut state = Self {
            storage,
            engine,
            config,
            server: None,
            starting: false,
            error: None,
            _starting: None,
        };
        if state.config.enabled && this_build::serves_mcp() {
            state.start_listener(cx);
        }
        state
    }

    pub fn port(&self) -> u16 {
        self.config.port
    }

    pub fn set_port(&mut self, port: u16, cx: &mut Context<Self>) {
        if port == 0 || self.config.port == port {
            return;
        }
        self.config.port = port;
        mcp_local::write(&self.config);
        cx.notify();
    }

    pub fn token(&self) -> &str {
        &self.config.token
    }

    pub fn rotate_token(&mut self, cx: &mut Context<Self>) {
        self.config.token = generate_local_token();
        mcp_local::write(&self.config);
        self.sync_access();
        cx.notify();
    }

    pub fn can_send(&self) -> bool {
        self.config.can_send
    }

    pub fn set_can_send(&mut self, can_send: bool, cx: &mut Context<Self>) {
        if self.config.can_send == can_send {
            return;
        }
        self.config.can_send = can_send;
        mcp_local::write(&self.config);
        self.sync_access();
        cx.notify();
    }

    pub fn can_write(&self) -> bool {
        self.config.can_write
    }

    pub fn set_can_write(&mut self, can_write: bool, cx: &mut Context<Self>) {
        if self.config.can_write == can_write {
            return;
        }
        self.config.can_write = can_write;
        mcp_local::write(&self.config);
        self.sync_access();
        cx.notify();
    }

    fn sync_access(&self) {
        if let Some(server) = &self.server {
            server.set_access(self.config.token.clone(), self.config.can_send, self.config.can_write);
        }
    }

    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    pub fn starting(&self) -> bool {
        self.starting
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The MCP endpoint URL (`http://127.0.0.1:4020/v1/mcp`).
    pub fn endpoint(&self) -> String {
        match &self.server {
            Some(server) => server.endpoint(),
            None => format!("http://127.0.0.1:{}/v1/mcp", self.config.port),
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.server.is_some() || self.starting {
            return;
        }
        self.config.enabled = true;
        mcp_local::write(&self.config);
        self.start_listener(cx);
        cx.notify();
    }

    fn start_listener(&mut self, cx: &mut Context<Self>) {
        self.starting = true;
        self.error = None;
        let starting = this_build::start_local_mcp(
            self.config.port,
            self.config.token.clone(),
            self.config.can_send,
            self.config.can_write,
            self.storage.clone(),
            self.engine.clone(),
            cx,
        );
        self._starting = Some(cx.spawn(async move |this, cx| {
            let started = starting.await;
            let _ = this.update(cx, |this, cx| {
                this.starting = false;
                match started {
                    Ok(server) => {
                        this.server = Some(server);
                    }
                    Err(why) => {
                        this.error = Some(why);
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.config.enabled = false;
        mcp_local::write(&self.config);
        self._starting = None;
        self.starting = false;
        self.server = None;
        self.error = None;
        cx.notify();
    }
}
