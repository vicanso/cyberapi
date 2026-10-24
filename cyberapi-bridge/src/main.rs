//! `cyberapi-bridge`: serve CyberAPI's browser build.
//!
//! ```text
//! CYBERAPI_BRIDGE_STORE=postgres://… cyberapi-bridge --listen 127.0.0.1:7380
//! ```

use clap::Parser;
use cyberapi_bridge::Bridge;
use cyberapi_http::{NativeEngine, TargetPolicy};
use cyberapi_store_sql::SqlStore;
use ipnet::IpNet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use tracing_subscriber::EnvFilter;

/// What `--version` and the start say: the release's version or a nightly's
/// own, with the commit (#54).
fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(cyberapi_core::version::describe)
}

#[derive(Parser)]
#[command(version = version(), about = "The server of CyberAPI's browser build")]
struct Args {
    /// Where to listen. The image listens on every interface.
    #[arg(long, env = "CYBERAPI_BRIDGE_LISTEN", default_value = "127.0.0.1:7380")]
    listen: SocketAddr,
    /// The database: `postgres://…` or `mysql://…`. An environment variable
    /// rather than a flag, which anyone who can list processes would read.
    #[arg(long, env = "CYBERAPI_BRIDGE_STORE", hide_env_values = true)]
    store: String,
    /// Serve this directory as the page instead of the one compiled in
    /// (`cyberapi-web/www` after `make web-bundle`).
    #[arg(long)]
    www: Option<PathBuf>,
    /// Serve the page and the API under this path, e.g. `/cyberapi`, for a
    /// reverse proxy that forwards it with the prefix kept. The login
    /// cookie is kept to it too.
    #[arg(long, env = "CYBERAPI_BRIDGE_BASE_PATH", default_value = "")]
    base_path: String,
    /// The token the first administrator is made with. Random, and logged,
    /// when not given; it counts only while the bridge has no account.
    #[arg(long, env = "CYBERAPI_BRIDGE_SETUP_TOKEN", hide_env_values = true)]
    setup_token: Option<String>,
    /// Leave `Secure` off the login cookie, for a bridge reached over plain
    /// HTTP on this machine.
    #[arg(long)]
    insecure_cookie: bool,
    /// Address ranges requests may be sent to besides those the policy
    /// reaches, e.g. `127.0.0.1/32` for a service on this host, or
    /// `10.1.0.0/16` under `--public-only`.
    #[arg(long = "allow-target", env = "CYBERAPI_BRIDGE_ALLOW_TARGETS", value_delimiter = ',')]
    allow_targets: Vec<IpNet>,
    /// Send to public addresses only, refusing the private ranges too: for
    /// a bridge open to the internet. Without it, requests reach public and
    /// private networks, never this host (loopback) or its link (link-local,
    /// the cloud metadata address among them).
    #[arg(long, env = "CYBERAPI_BRIDGE_PUBLIC_ONLY")]
    public_only: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let args = Args::parse();
    tracing::info!(version = version(), "cyberapi-bridge starting");
    let base_path = cyberapi_bridge::normalize_base_path(&args.base_path)?;

    let store = SqlStore::connect(&args.store).await?;
    let setup_token = match args.setup_token {
        Some(token) => token,
        None => cyberapi_bridge::new_setup_token()?,
    };
    if store.account_count().await? == 0 {
        tracing::info!(%setup_token, "no account yet: make the first administrator with this setup token");
    }
    if args.public_only {
        tracing::info!("requests go to public addresses only");
    } else {
        tracing::info!("requests go to public and private networks, not to this host or link-local addresses");
    }
    if !args.allow_targets.is_empty() {
        tracing::info!(allowed = ?args.allow_targets, "requests may also go to these ranges");
    }
    let allowed = args.allow_targets;
    let policy = if args.public_only {
        TargetPolicy::PublicOnly { allowed }
    } else {
        TargetPolicy::Networks { allowed }
    };
    let engine = NativeEngine::new(tokio::runtime::Handle::current(), policy);
    let bridge = Bridge::new(
        store,
        Arc::new(engine),
        setup_token,
        !args.insecure_cookie,
        base_path.clone(),
    );
    bridge.spawn_housekeeping();
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    let at = format!("{base_path}/");
    match &args.www {
        Some(www) => tracing::info!(listen = %args.listen, %at, www = %www.display(), "cyberapi-bridge serving"),
        None => tracing::info!(listen = %args.listen, %at, "cyberapi-bridge serving, with the page built in"),
    }
    // The peer's address, for counting failed sign-ins by where they come
    // from. Behind a proxy that is the proxy's.
    let app = bridge
        .router(args.www)
        .into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app).with_graceful_shutdown(stopped()).await?;
    Ok(())
}

/// Ctrl-C, or the SIGTERM a service manager or `docker stop` sends. In a
/// container the bridge is PID 1, which a signal without a handler does not
/// stop. Requests under way finish first.
async fn stopped() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = terminated() => {}
    }
    tracing::info!("cyberapi-bridge stopping");
}

#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            terminate.recv().await;
        }
        Err(error) => {
            tracing::warn!(%error, "SIGTERM cannot be handled");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn terminated() {
    std::future::pending::<()>().await;
}
