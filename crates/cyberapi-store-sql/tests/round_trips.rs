//! How many statements a store call sends: each is a round trip to the
//! database, and a big project's sync made thousands (PERF-7). The same
//! environment as `conformance.rs`: a run without the databases skips.

use cyberapi_core::import::ImportItem;
use cyberapi_core::model::HttpRequest;
use cyberapi_store::NewCollection;
use cyberapi_store_sql::{NewAccount, SqlStore};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts the statements sqlx logs. Set for this thread alone: a test on
/// tokio's single thread sees its own calls and no other test's.
struct Statements(Arc<AtomicUsize>);

impl tracing::Subscriber for Statements {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() == "sqlx::query" {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// How many APIs the project has: more than any fixed number of
/// statements a sync may send.
const APIS: usize = 120;

async fn a_sync_writes_what_changed(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url).await.expect("connect");
    let account = store
        .create_account(NewAccount {
            name: format!("round-trips-{}", ulid::Ulid::generate()),
            display_name: "Round trips".into(),
            password_hash: "unused".into(),
            admin: false,
        })
        .await
        .expect("create an account");
    let storage = store.scoped(account);
    let collection = storage
        .collections()
        .create(NewCollection {
            name: "big".into(),
            ..Default::default()
        })
        .await
        .expect("create");
    let items = (0..APIS)
        .map(|n| ImportItem::Api {
            name: format!("api {n}"),
            request: Box::new(HttpRequest {
                uri: format!("https://a.test/{n}"),
                ..Default::default()
            }),
        })
        .collect();
    storage
        .tree()
        .import(collection.id.clone(), None, items)
        .await
        .expect("import");
    let read = storage.tree().config(collection.id.clone()).await.expect("config");
    let mut changed = read.config.clone();
    changed.apis[7].name = "renamed".into();

    let statements = Arc::new(AtomicUsize::new(0));
    let counting = tracing::subscriber::set_default(Statements(statements.clone()));
    let revision = storage
        .tree()
        .replace(collection.id.clone(), changed.clone(), Some(read.revision))
        .await
        .expect("replace");
    drop(counting);
    let sent = statements.load(Ordering::Relaxed);
    assert!(sent > 0, "the statements are counted");
    assert!(
        sent < APIS / 2,
        "a sync of {APIS} APIs, one changed, sent {sent} statements"
    );
    let now = storage.tree().config(collection.id.clone()).await.expect("config");
    assert_eq!((now.revision, now.config), (revision, changed));
}

#[tokio::test]
async fn postgres_syncs_in_few_round_trips() {
    a_sync_writes_what_changed("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_syncs_in_few_round_trips() {
    a_sync_writes_what_changed("CYBERAPI_TEST_MYSQL").await;
}
