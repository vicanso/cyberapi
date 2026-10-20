//! The bridge's store (`docs/architecture.md` §4.4, §9): every Store trait
//! over PostgreSQL or MySQL, for many accounts.
//!
//! [`SqlStore`] is the database; [`SqlStore::scoped`] is one account's view
//! of it, and every call through that view is checked against what the
//! account may do — a project it cannot see is "not found", a change its role
//! does not allow is "forbidden". The traits never name an account, so the
//! desktop and the bridge speak the same ones.

mod rows;
mod scoped;
mod sql;

pub use sql::Dialect;

use chrono::Utc;
use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::{AccountId, CollectionId, TeamId, Timestamp};
use cyberapi_store::{Storage, StoreError};
use scoped::Scoped;
use sqlx::any::{AnyPoolOptions, install_default_drivers};
use sqlx::migrate::MigrateError;
use sqlx::{AnyPool, Row};
use std::sync::Arc;
use std::time::Duration;
use ulid::Ulid;

/// Connections one process keeps to the database.
const POOL_SIZE: u32 = 10;
/// How long a call waits for one.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn db(error: impl std::fmt::Display) -> StoreError {
    StoreError::backend(error)
}

pub(crate) fn new_id() -> String {
    Ulid::generate().to_string()
}

/// "Now", to the second: what is answered is exactly what a read of the row
/// gives back.
pub(crate) fn now() -> Timestamp {
    Timestamp::parse_lenient(&Timestamp::new(Utc::now().fixed_offset()).to_rfc3339())
}

/// The fields of a new account. The password is hashed by the caller; the
/// store never sees it.
#[derive(Clone, Debug, Default)]
pub struct NewAccount {
    pub name: String,
    pub display_name: String,
    pub password_hash: String,
    pub admin: bool,
}

#[derive(Clone)]
pub struct SqlStore {
    pool: AnyPool,
    dialect: Dialect,
}

impl SqlStore {
    /// Connects to `url` (`postgres://…` or `mysql://…`) and brings the
    /// schema up to this build's.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(POOL_SIZE)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect(url)
            .await
            .map_err(|e| db(format!("{}: {e}", redacted(url))))?;
        let dialect = {
            let conn = pool.acquire().await.map_err(db)?;
            Dialect::of(conn.backend_name()).ok_or_else(|| {
                db(format!(
                    "the store needs PostgreSQL or MySQL, not {}",
                    conn.backend_name()
                ))
            })?
        };
        let store = Self { pool, dialect };
        store.migrate().await?;
        Ok(store)
    }

    /// A step cut short — MySQL cannot take back the half of one that ran,
    /// so sqlx marks it unfinished and refuses to go on — is run again:
    /// every statement of a step is written to run twice safely. Once: a
    /// step that fails again says why.
    async fn migrate(&self) -> Result<(), StoreError> {
        let migrator = match self.dialect {
            Dialect::Postgres => sqlx::migrate!("migrations/postgres"),
            Dialect::MySql => sqlx::migrate!("migrations/mysql"),
        };
        let failed = |e: MigrateError| db(format!("the store's schema could not be brought up to date: {e}"));
        match migrator.run(&self.pool).await {
            Err(MigrateError::Dirty(version)) => {
                tracing::warn!(version, "a schema step was cut short; running it again");
                sqlx::query(self.dialect.sql("DELETE FROM _sqlx_migrations WHERE version = $1"))
                    .bind(version)
                    .execute(&self.pool)
                    .await
                    .map_err(db)?;
                migrator.run(&self.pool).await.map_err(failed)
            }
            done => done.map_err(failed),
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// One account's stores. Each call checks what the account may do.
    pub fn scoped(&self, account: AccountId) -> Storage {
        Storage::from_backend(Arc::new(Scoped::new(self.clone(), account)))
    }

    /// Creates an account with its personal space (§9.1), whose only member
    /// it is, as owner.
    pub async fn create_account(&self, input: NewAccount) -> Result<AccountId, StoreError> {
        let account = AccountId::new(new_id());
        let team = TeamId::new(new_id());
        let at = now().to_rfc3339();
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query(self.dialect.sql(sql::ACCOUNT_INSERT))
            .bind(account.as_str().to_string())
            .bind(input.name.clone())
            .bind(input.display_name.clone())
            .bind(input.password_hash)
            .bind(i32::from(input.admin))
            .bind(at.clone())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let space = if input.display_name.trim().is_empty() {
            input.name
        } else {
            input.display_name
        };
        sqlx::query(self.dialect.sql(sql::TEAM_INSERT))
            .bind(team.as_str().to_string())
            .bind(space)
            .bind(Some(account.as_str().to_string()))
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(self.dialect.pick(&sql::MEMBER_UPSERT))
            .bind(team.as_str().to_string())
            .bind(account.as_str().to_string())
            .bind(TeamRole::Owner.as_str())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(account)
    }

    /// A team, with its creator as owner.
    pub async fn create_team(&self, name: &str, creator: &AccountId) -> Result<TeamId, StoreError> {
        let team = TeamId::new(new_id());
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query(self.dialect.sql(sql::TEAM_INSERT))
            .bind(team.as_str().to_string())
            .bind(name.to_string())
            .bind(None::<String>)
            .bind(now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(self.dialect.pick(&sql::MEMBER_UPSERT))
            .bind(team.as_str().to_string())
            .bind(creator.as_str().to_string())
            .bind(TeamRole::Owner.as_str())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(team)
    }

    /// Puts an account in a team with `role`, or takes it out (`None`).
    pub async fn set_member(
        &self,
        team: &TeamId,
        account: &AccountId,
        role: Option<TeamRole>,
    ) -> Result<(), StoreError> {
        let query = match role {
            Some(role) => sqlx::query(self.dialect.pick(&sql::MEMBER_UPSERT))
                .bind(team.as_str().to_string())
                .bind(account.as_str().to_string())
                .bind(role.as_str()),
            None => sqlx::query(self.dialect.sql(sql::MEMBER_DELETE))
                .bind(team.as_str().to_string())
                .bind(account.as_str().to_string()),
        };
        query.execute(&self.pool).await.map_err(db)?;
        Ok(())
    }

    /// Grants a project to an account or a team with `role`, or takes the
    /// grant back (`None`).
    pub async fn set_grant(
        &self,
        collection: &CollectionId,
        grantee: &Grantee,
        role: Option<CollectionRole>,
    ) -> Result<(), StoreError> {
        let (kind, id) = match grantee {
            Grantee::Account(account) => ("account", account.as_str()),
            Grantee::Team(team) => ("team", team.as_str()),
        };
        let query = match role {
            Some(role) => sqlx::query(self.dialect.pick(&sql::GRANT_UPSERT))
                .bind(collection.as_str().to_string())
                .bind(kind)
                .bind(id.to_string())
                .bind(role.as_str()),
            None => sqlx::query(self.dialect.sql(sql::GRANT_DELETE))
                .bind(collection.as_str().to_string())
                .bind(kind)
                .bind(id.to_string()),
        };
        query.execute(&self.pool).await.map_err(db)?;
        Ok(())
    }

    /// How many rows anything holds for a project, for the conformance probe.
    #[cfg(feature = "conformance")]
    async fn rows_of(&self, collection: &CollectionId) -> Result<usize, StoreError> {
        let mut total = 0;
        for (index, statement) in sql::PROBE_COUNTS.iter().enumerate() {
            let key = if index == sql::PROBE_COUNTS.len() - 1 {
                scoped::workspace_scope(collection)
            } else {
                collection.as_str().to_string()
            };
            let row = sqlx::query(self.dialect.sql(statement))
                .bind(key)
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
            total += usize::try_from(row.try_get::<i64, _>(0).map_err(db)?).unwrap_or(0);
        }
        Ok(total)
    }
}

/// The store's probe, for any account: rows do not belong to one.
#[cfg(feature = "conformance")]
impl cyberapi_store::conformance::Probe for SqlStore {
    fn rows_of(&self, collection: CollectionId) -> cyberapi_store::StoreFuture<usize> {
        let store = self.clone();
        Box::pin(async move { store.rows_of(&collection).await })
    }
}

/// A database URL without its password, for messages.
fn redacted(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            let credentials = &url[scheme + 3..at];
            match credentials.split_once(':') {
                Some((user, _)) => format!("{}{user}:***{}", &url[..scheme + 3], &url[at..]),
                None => url.to_string(),
            }
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_stay_out_of_messages() {
        assert_eq!(redacted("postgres://u:secret@db:5432/x"), "postgres://u:***@db:5432/x");
        assert_eq!(redacted("mysql://db/x"), "mysql://db/x");
    }
}
