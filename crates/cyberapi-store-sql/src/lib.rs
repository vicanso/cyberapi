//! The bridge's store (`docs/architecture.md` §4.4, §9): every Store trait
//! over PostgreSQL or MySQL, for many accounts.
//!
//! [`SqlStore`] is the database; [`SqlStore::scoped`] is one account's view
//! of it, and every call through that view is checked against what the
//! account may do — a project it cannot see is "not found", a change its role
//! does not allow is "forbidden". The traits never name an account, so the
//! desktop and the bridge speak the same ones. [`SqlStore::directory`] is the
//! account's view of the accounts and teams themselves (§9.4).

mod directory;
mod rows;
mod scoped;
mod sql;

pub use directory::Passwords;
pub use sql::Dialect;

use chrono::Utc;
use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::{AccountId, CollectionId, TeamId, Timestamp};
use cyberapi_store::{DirectoryStore, Storage, StoreError};
use directory::NoPasswords;
use scoped::Scoped;
use sqlx::any::{AnyPoolOptions, install_default_drivers};
use sqlx::migrate::MigrateError;
use sqlx::{AnyConnection, AnyPool, Row};
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

/// A signed-in account, as the bridge sees it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub display_name: String,
    pub admin: bool,
}

fn account(row: &sqlx::any::AnyRow) -> Result<Account, StoreError> {
    Ok(Account {
        id: AccountId::new(rows::text(row, 0)?),
        name: rows::text(row, 1)?,
        display_name: rows::text(row, 2)?,
        admin: row.try_get::<i32, _>(3).map_err(db)? != 0,
    })
}

#[derive(Clone)]
pub struct SqlStore {
    pool: AnyPool,
    dialect: Dialect,
    passwords: Arc<dyn Passwords>,
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
        let store = Self {
            pool,
            dialect,
            passwords: Arc::new(NoPasswords),
        };
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

    /// How the directory sets and checks passwords; until this, it does
    /// neither.
    pub fn with_passwords(self, passwords: Arc<dyn Passwords>) -> Self {
        Self { passwords, ..self }
    }

    /// One account's stores. Each call checks what the account may do.
    pub fn scoped(&self, account: AccountId) -> Storage {
        Storage::from_backend(Arc::new(Scoped::new(self.clone(), account)))
    }

    /// One account's view of the accounts and teams (§9.4).
    pub fn directory(&self, account: AccountId) -> Arc<dyn DirectoryStore> {
        Arc::new(Scoped::new(self.clone(), account))
    }

    /// Creates an account with its personal space (§9.1), whose only member
    /// it is, as owner.
    pub async fn create_account(&self, input: NewAccount) -> Result<AccountId, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let account = self.insert_account(&mut tx, input).await?;
        tx.commit().await.map_err(db)?;
        Ok(account)
    }

    /// The first administrator (§9.5), made only while there is no account
    /// at all: `None` once there is one — or when another setup, at the
    /// same moment, got there first.
    pub async fn create_first_admin(&self, input: NewAccount) -> Result<Option<AccountId>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let claimed = sqlx::query(self.dialect.pick(&sql::SETUP_CLAIM))
            .bind(now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected()
            == 1;
        let accounts = sqlx::query(self.dialect.sql(sql::ACCOUNT_COUNT))
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?
            .try_get::<i64, _>(0)
            .map_err(db)?;
        if !claimed || accounts > 0 {
            tx.rollback().await.map_err(db)?;
            return Ok(None);
        }
        let account = self
            .insert_account(&mut tx, NewAccount { admin: true, ..input })
            .await?;
        tx.commit().await.map_err(db)?;
        Ok(Some(account))
    }

    async fn insert_account(&self, conn: &mut AnyConnection, input: NewAccount) -> Result<AccountId, StoreError> {
        let account = AccountId::new(new_id());
        let team = TeamId::new(new_id());
        let at = now().to_rfc3339();
        sqlx::query(self.dialect.sql(sql::ACCOUNT_INSERT))
            .bind(account.as_str().to_string())
            .bind(input.name.clone())
            .bind(input.display_name.clone())
            .bind(input.password_hash)
            .bind(i32::from(input.admin))
            .bind(at.clone())
            .execute(&mut *conn)
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
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        sqlx::query(self.dialect.pick(&sql::MEMBER_UPSERT))
            .bind(team.as_str().to_string())
            .bind(account.as_str().to_string())
            .bind(TeamRole::Owner.as_str())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        Ok(account)
    }

    pub async fn account_count(&self) -> Result<u64, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::ACCOUNT_COUNT))
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(u64::try_from(row.try_get::<i64, _>(0).map_err(db)?).unwrap_or(0))
    }

    /// An enabled account by its sign-in name, with its password hash.
    pub async fn find_login(&self, name: &str) -> Result<Option<(Account, String)>, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::ACCOUNT_LOGIN))
            .bind(name.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(|row| Ok((account(&row)?, rows::text(&row, 4)?))).transpose()
    }

    /// A login that lasts until `expires_at` (unix seconds). The store keeps
    /// only the token's hash.
    pub async fn create_session(
        &self,
        token_hash: &str,
        account: &AccountId,
        expires_at: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(self.dialect.sql(sql::SESSION_INSERT))
            .bind(token_hash.to_string())
            .bind(account.as_str().to_string())
            .bind(expires_at)
            .bind(now().to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    /// Whose login `token_hash` is, while it lasts (`now` in unix seconds)
    /// and the account is enabled.
    pub async fn session_account(&self, token_hash: &str, now: i64) -> Result<Option<Account>, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::SESSION_ACCOUNT))
            .bind(token_hash.to_string())
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.map(|row| account(&row)).transpose()
    }

    /// Forgets the logins that ran out by `now` (unix seconds); how many.
    pub async fn delete_expired_sessions(&self, now: i64) -> Result<u64, StoreError> {
        let done = sqlx::query(self.dialect.sql(sql::SESSIONS_EXPIRED_DELETE))
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(done.rows_affected())
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), StoreError> {
        sqlx::query(self.dialect.sql(sql::SESSION_DELETE))
            .bind(token_hash.to_string())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
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
