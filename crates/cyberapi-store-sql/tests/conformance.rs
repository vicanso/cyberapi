//! The shared rules, on each database the store supports.
//!
//! The databases come from the environment, so a run without them skips
//! rather than fails: `make db-up` starts both in local containers, and
//! `make test-sql` runs these against them.
//!
//! - `CYBERAPI_TEST_POSTGRES`, e.g. `postgres://cyberapi:cyberapi@127.0.0.1:55432/cyberapi`
//! - `CYBERAPI_TEST_MYSQL`, e.g. `mysql://cyberapi:cyberapi@127.0.0.1:53306/cyberapi`

use cyberapi_store::conformance;
use cyberapi_store_sql::{NewAccount, SqlStore};

async fn conforms(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url).await.expect("connect");
    // A new account each run: the checks share the database with earlier
    // runs and only look at what they make.
    let account = store
        .create_account(NewAccount {
            name: format!("conformance-{}", ulid::Ulid::generate()),
            display_name: "Conformance".into(),
            password_hash: "unused".into(),
            admin: false,
        })
        .await
        .expect("create an account");
    conformance::run(&store.scoped(account), &store).await;
}

#[tokio::test]
async fn postgres_conforms() {
    conforms("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_conforms() {
    conforms("CYBERAPI_TEST_MYSQL").await;
}
