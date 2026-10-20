//! `cyberapi-bridge`: serve CyberAPI's browser build.
//!
//! ```text
//! CYBERAPI_BRIDGE_STORE=postgres://… cyberapi-bridge --listen 127.0.0.1:7380 --www cyberapi-web/www
//! ```

use clap::Parser;
use cyberapi_bridge::Bridge;
use cyberapi_store_sql::SqlStore;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "The server of CyberAPI's browser build")]
struct Args {
    /// Where to listen.
    #[arg(long, default_value = "127.0.0.1:7380")]
    listen: SocketAddr,
    /// The database: `postgres://…` or `mysql://…`. An environment variable
    /// rather than a flag, which anyone who can list processes would read.
    #[arg(long, env = "CYBERAPI_BRIDGE_STORE", hide_env_values = true)]
    store: String,
    /// The page's files: `cyberapi-web/www` after `make web-bundle`.
    #[arg(long, default_value = "cyberapi-web/www")]
    www: PathBuf,
    /// The token the first administrator is made with. Random, and logged,
    /// when not given; it counts only while the bridge has no account.
    #[arg(long, env = "CYBERAPI_BRIDGE_SETUP_TOKEN", hide_env_values = true)]
    setup_token: Option<String>,
    /// Leave `Secure` off the login cookie, for a bridge reached over plain
    /// HTTP on this machine.
    #[arg(long)]
    insecure_cookie: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let args = Args::parse();

    let store = SqlStore::connect(&args.store).await?;
    let setup_token = match args.setup_token {
        Some(token) => token,
        None => cyberapi_bridge::new_setup_token()?,
    };
    if store.account_count().await? == 0 {
        tracing::info!(%setup_token, "no account yet: make the first administrator with this setup token");
    }
    let bridge = Bridge::new(store, setup_token, !args.insecure_cookie);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, www = %args.www.display(), "cyberapi-bridge serving");
    axum::serve(listener, bridge.router(Some(args.www))).await?;
    Ok(())
}
