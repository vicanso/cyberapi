//! One account's directory (§9.4): accounts, teams and who has which
//! project, with the rules of who may change them. A team the account is
//! not in is "not found", as a project is.

use crate::scoped::Scoped;
use crate::sql;
use crate::{db, new_id, new_token, now, rows, token_hash, unix_now};
use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::directory::{
    AccountInfo, Grant, INVITE_DAYS, Invite, IssuedMcpToken, MCP_TOKEN_DAYS, MCP_TOKEN_PREFIX, McpToken, Me, Member,
    NewInvite, NewMcpToken, Team, TeamPlace, invite_needs, sort_teams, valid_password, valid_token_name,
};
use cyberapi_core::{AccountId, CollectionId, InviteId, McpTokenId, TeamId, Timestamp};
use cyberapi_store::rules::required_name;
use cyberapi_store::{AccountEdit, DirectoryStore, Entity, Refusal, StoreError, StoreFuture};
use sqlx::any::AnyRow;
use sqlx::{AnyConnection, Row as _};

/// How passwords are kept. The bridge hashes them (Argon2id) and checks
/// them; the store keeps only what it is handed back.
pub trait Passwords: Send + Sync {
    fn hash(&self, password: &str) -> Result<String, String>;
    fn verify(&self, hash: &str, password: &str) -> bool;
}

/// Until the bridge says how, no password is set or matches.
pub(crate) struct NoPasswords;

impl Passwords for NoPasswords {
    fn hash(&self, _: &str) -> Result<String, String> {
        Err("this store was given no way to hash passwords".into())
    }

    fn verify(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn refused(reason: Refusal) -> StoreError {
    StoreError::Refused { reason }
}

fn flag(row: &AnyRow, index: usize) -> Result<bool, StoreError> {
    Ok(row.try_get::<i32, _>(index).map_err(db)? != 0)
}

/// `id, name, display_name, admin, disabled` from `start`.
fn account_info(row: &AnyRow, start: usize) -> Result<AccountInfo, StoreError> {
    Ok(AccountInfo {
        id: AccountId::new(rows::text(row, start)?),
        name: rows::text(row, start + 1)?,
        display_name: rows::text(row, start + 2)?,
        admin: flag(row, start + 3)?,
        disabled: flag(row, start + 4)?,
    })
}

/// `id, name, personal_of, role`.
fn team(row: &AnyRow) -> Result<Team, StoreError> {
    Ok(Team {
        id: TeamId::new(rows::text(row, 0)?),
        name: rows::text(row, 1)?,
        personal: row.try_get::<Option<String>, _>(2).map_err(db)?.is_some(),
        role: TeamRole::parse(&rows::text(row, 3)?).unwrap_or(TeamRole::Member),
    })
}

fn team_role(text: &str) -> TeamRole {
    TeamRole::parse(text).unwrap_or(TeamRole::Member)
}

/// `id, name, can_send, created_at, expires_at, can_write`, from column
/// `start` on.
pub(crate) fn mcp_token(row: &AnyRow, start: usize) -> Result<McpToken, StoreError> {
    Ok(McpToken {
        id: McpTokenId::new(rows::text(row, start)?),
        name: rows::text(row, start + 1)?,
        can_send: row.try_get::<i32, _>(start + 2).map_err(db)? != 0,
        can_write: row.try_get::<i32, _>(start + 5).map_err(db)? != 0,
        created_at: Timestamp::parse_lenient(&rows::text(row, start + 3)?),
        expires_at: row
            .try_get::<Option<i64>, _>(start + 4)
            .map_err(db)?
            .map(Timestamp::from_unix),
    })
}

/// `id, team, team name, role, created_at, expires_at`.
fn invite(row: &AnyRow) -> Result<Invite, StoreError> {
    let optional = |index| row.try_get::<Option<String>, _>(index).map_err(db);
    let place = match (optional(1)?, optional(3)?) {
        (Some(team), Some(role)) => Some(TeamPlace {
            team: TeamId::new(team),
            role: team_role(&role),
        }),
        _ => None,
    };
    Ok(Invite {
        id: InviteId::new(rows::text(row, 0)?),
        place,
        team_name: optional(2)?,
        created_at: Timestamp::parse_lenient(&rows::text(row, 4)?),
        expires_at: Timestamp::from_unix(row.try_get::<i64, _>(5).map_err(db)?),
    })
}

async fn count(
    conn: &mut AnyConnection,
    statement: sqlx::AssertSqlSafe<std::borrow::Cow<'static, str>>,
    key: &str,
) -> Result<i64, StoreError> {
    sqlx::query(statement)
        .bind(key.to_string())
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?
        .try_get::<i64, _>(0)
        .map_err(db)
}

impl Scoped {
    async fn account_info(&self, conn: &mut AnyConnection, id: &AccountId) -> Result<AccountInfo, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::ACCOUNT_INFO))
            .bind(id.as_str().to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .ok_or_else(|| StoreError::not_found(Entity::Account, id))?;
        account_info(&row, 0)
    }

    /// The reader, when they are a system administrator — asked each time,
    /// so taking the role away takes effect at once.
    async fn require_admin(&self, conn: &mut AnyConnection) -> Result<AccountInfo, StoreError> {
        let me = self.account_info(conn, &self.account).await?;
        if me.admin {
            Ok(me)
        } else {
            Err(refused(Refusal::AdminOnly))
        }
    }

    /// The team, with the reader's place in it; "not found" when they are
    /// not in it.
    async fn team_of(&self, conn: &mut AnyConnection, team_id: &TeamId) -> Result<Team, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::TEAM_OF))
            .bind(team_id.as_str().to_string())
            .bind(self.account())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .ok_or_else(|| StoreError::not_found(Entity::Team, team_id))?;
        team(&row)
    }

    /// `password` hashed, off the async workers.
    async fn hash(&self, password: String) -> Result<String, StoreError> {
        self.store.hash_password(password).await
    }
}

impl DirectoryStore for Scoped {
    fn me(&self) -> StoreFuture<Me> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let account = this.account_info(&mut conn, &this.account).await?;
            let mut teams = sqlx::query(this.dialect.sql(sql::TEAMS_OF))
                .bind(this.account())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?
                .iter()
                .map(team)
                .collect::<Result<Vec<_>, _>>()?;
            sort_teams(&mut teams);
            Ok(Me { account, teams })
        })
    }

    fn accounts(&self) -> StoreFuture<Vec<AccountInfo>> {
        self.run(|this| async move {
            let mut accounts = sqlx::query(this.dialect.sql(sql::ACCOUNTS_ALL))
                .fetch_all(&this.pool)
                .await
                .map_err(db)?
                .iter()
                .map(|row| account_info(row, 0))
                .collect::<Result<Vec<_>, _>>()?;
            accounts.sort_by_key(|account| account.name.to_lowercase());
            Ok(accounts)
        })
    }

    fn create_team(&self, name: String) -> StoreFuture<Team> {
        self.run(|this| async move {
            let name = required_name(&name)?;
            let id = this.store.create_team(&name, &this.account).await?;
            Ok(Team {
                id,
                name,
                personal: false,
                role: TeamRole::Owner,
            })
        })
    }

    fn rename_team(&self, team_id: TeamId, name: String) -> StoreFuture<()> {
        self.run(|this| async move {
            let name = required_name(&name)?;
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let team = this.team_of(&mut conn, &team_id).await?;
            if team.personal {
                return Err(refused(Refusal::PersonalSpace));
            }
            if team.role < TeamRole::Admin {
                return Err(refused(Refusal::TeamRole {
                    needed: TeamRole::Admin,
                }));
            }
            sqlx::query(this.dialect.sql(sql::TEAM_RENAME))
                .bind(name)
                .bind(team_id.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn delete_team(&self, team_id: TeamId) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            let team = this.team_of(&mut tx, &team_id).await?;
            if team.personal {
                return Err(refused(Refusal::PersonalSpace));
            }
            if team.role < TeamRole::Owner {
                return Err(refused(Refusal::TeamRole {
                    needed: TeamRole::Owner,
                }));
            }
            let id = team_id.as_str().to_string();
            if count(&mut tx, this.dialect.sql(sql::TEAM_PROJECT_COUNT), &id).await? > 0 {
                return Err(refused(Refusal::TeamHasProjects));
            }
            // Its members go with it; grants to it name it only by id.
            for statement in [sql::TEAM_GRANTS_DELETE, sql::TEAM_DELETE] {
                sqlx::query(this.dialect.sql(statement))
                    .bind(id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            tx.commit().await.map_err(db)
        })
    }

    fn members(&self, team_id: TeamId) -> StoreFuture<Vec<Member>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.team_of(&mut conn, &team_id).await?;
            let mut members = sqlx::query(this.dialect.sql(sql::TEAM_MEMBERS))
                .bind(team_id.as_str().to_string())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?
                .iter()
                .map(|row| {
                    Ok(Member {
                        account: account_info(row, 0)?,
                        role: team_role(&rows::text(row, 5)?),
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            // Owners first, then admins, then members; by name within each.
            members.sort_by(|a, b| {
                b.role
                    .cmp(&a.role)
                    .then_with(|| a.account.label().to_lowercase().cmp(&b.account.label().to_lowercase()))
            });
            Ok(members)
        })
    }

    fn set_member(&self, team_id: TeamId, account: AccountId, role: Option<TeamRole>) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            let id = team_id.as_str().to_string();
            sqlx::query(this.dialect.sql(sql::TEAM_LOCK))
                .bind(id.clone())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
            let mine = this.team_of(&mut tx, &team_id).await?;
            if mine.personal {
                return Err(refused(Refusal::PersonalSpace));
            }
            let theirs = sqlx::query(this.dialect.sql(sql::TEAM_ROLE))
                .bind(id.clone())
                .bind(account.as_str().to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
                .map(|row| rows::text(&row, 0))
                .transpose()?
                .map(|role| team_role(&role));
            let leaving = account == this.account && role.is_none();
            if !leaving {
                if mine.role < TeamRole::Admin {
                    return Err(refused(Refusal::TeamRole {
                        needed: TeamRole::Admin,
                    }));
                }
                // Only an owner makes an owner, or changes one.
                let touches_owner = role == Some(TeamRole::Owner) || theirs == Some(TeamRole::Owner);
                if touches_owner && mine.role < TeamRole::Owner {
                    return Err(refused(Refusal::TeamRole {
                        needed: TeamRole::Owner,
                    }));
                }
            }
            if role.is_some() && theirs.is_none() {
                this.account_info(&mut tx, &account).await?;
            }
            if theirs == Some(TeamRole::Owner)
                && role != Some(TeamRole::Owner)
                && count(&mut tx, this.dialect.sql(sql::TEAM_OWNER_COUNT), &id).await? <= 1
            {
                return Err(refused(Refusal::LastOwner));
            }
            let query = match role {
                Some(role) => sqlx::query(this.dialect.pick(&sql::MEMBER_UPSERT))
                    .bind(id)
                    .bind(account.as_str().to_string())
                    .bind(role.as_str()),
                None => sqlx::query(this.dialect.sql(sql::MEMBER_DELETE))
                    .bind(id)
                    .bind(account.as_str().to_string()),
            };
            query.execute(&mut *tx).await.map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }

    fn grants(&self, collection: CollectionId) -> StoreFuture<Vec<Grant>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Manager).await?;
            let mut grants = sqlx::query(this.dialect.sql(sql::GRANTS_OF))
                .bind(collection.as_str().to_string())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?
                .iter()
                .map(|row| {
                    let id = rows::text(row, 1)?;
                    let text = |index| row.try_get::<Option<String>, _>(index).map_err(db);
                    let (grantee, name) = if rows::text(row, 0)? == "team" {
                        (Grantee::Team(TeamId::new(id)), text(5)?.unwrap_or_default())
                    } else {
                        let info = AccountInfo {
                            name: text(3)?.unwrap_or_default(),
                            display_name: text(4)?.unwrap_or_default(),
                            ..Default::default()
                        };
                        (Grantee::Account(AccountId::new(id)), info.label().to_string())
                    };
                    Ok(Grant {
                        grantee,
                        name,
                        role: CollectionRole::parse(&rows::text(row, 2)?).unwrap_or(CollectionRole::Viewer),
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            grants.sort_by_key(|grant| grant.name.to_lowercase());
            Ok(grants)
        })
    }

    fn set_grant(&self, collection: CollectionId, grantee: Grantee, role: Option<CollectionRole>) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Manager).await?;
            if role.is_some() {
                match &grantee {
                    Grantee::Account(account) => {
                        this.account_info(&mut conn, account).await?;
                    }
                    Grantee::Team(team) => {
                        if count(&mut conn, this.dialect.sql(sql::TEAM_EXISTS), team.as_str()).await? == 0 {
                            return Err(StoreError::not_found(Entity::Team, team));
                        }
                    }
                }
            }
            drop(conn);
            this.store.set_grant(&collection, &grantee, role).await
        })
    }

    fn transfer(&self, collection: CollectionId, team_id: TeamId) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Manager).await?;
            this.team_of(&mut conn, &team_id).await?;
            sqlx::query(this.dialect.sql(sql::COLLECTION_TRANSFER))
                .bind(team_id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn invites(&self) -> StoreFuture<Vec<Invite>> {
        self.run(|this| async move {
            let mut invites = sqlx::query(this.dialect.sql(sql::INVITES_OPEN_OF))
                .bind(this.account())
                .bind(unix_now())
                .fetch_all(&this.pool)
                .await
                .map_err(db)?
                .iter()
                .map(invite)
                .collect::<Result<Vec<_>, _>>()?;
            invites.sort_by_key(|invite| invite.created_at);
            Ok(invites)
        })
    }

    fn create_invite(&self, place: Option<TeamPlace>) -> StoreFuture<NewInvite> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let team_name = match &place {
                None => {
                    this.require_admin(&mut conn).await?;
                    None
                }
                Some(place) => {
                    let team = this.team_of(&mut conn, &place.team).await?;
                    if team.personal {
                        return Err(refused(Refusal::PersonalSpace));
                    }
                    let needed = invite_needs(place.role);
                    if team.role < needed {
                        return Err(refused(Refusal::TeamRole { needed }));
                    }
                    Some(team.name)
                }
            };
            let token = new_token()?;
            let id = InviteId::new(new_id());
            let created_at = now();
            let expires_at = unix_now() + INVITE_DAYS * 24 * 60 * 60;
            sqlx::query(this.dialect.sql(sql::INVITE_INSERT))
                .bind(id.as_str().to_string())
                .bind(token_hash(&token))
                .bind(this.account())
                .bind(place.as_ref().map(|place| place.team.as_str().to_string()))
                .bind(place.as_ref().map(|place| place.role.as_str()))
                .bind(created_at.to_rfc3339())
                .bind(expires_at)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(NewInvite {
                invite: Invite {
                    id,
                    place,
                    team_name,
                    created_at,
                    expires_at: Timestamp::from_unix(expires_at),
                },
                token,
            })
        })
    }

    fn revoke_invite(&self, id: InviteId) -> StoreFuture<()> {
        self.run(|this| async move {
            let revoked = sqlx::query(this.dialect.sql(sql::INVITE_REVOKE))
                .bind(id.as_str().to_string())
                .bind(this.account())
                .execute(&this.pool)
                .await
                .map_err(db)?
                .rows_affected();
            if revoked == 0 {
                return Err(StoreError::not_found(Entity::Invite, &id));
            }
            Ok(())
        })
    }

    fn update_account(&self, account: AccountId, edit: AccountEdit) -> StoreFuture<AccountInfo> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require_admin(&mut conn).await?;
            if account == this.account && (!edit.admin || edit.disabled) {
                return Err(refused(Refusal::NotYourself));
            }
            this.account_info(&mut conn, &account).await?;
            sqlx::query(this.dialect.sql(sql::ACCOUNT_UPDATE))
                .bind(edit.display_name.trim().to_string())
                .bind(i32::from(edit.admin))
                .bind(i32::from(edit.disabled))
                .bind(account.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            if edit.disabled {
                sqlx::query(this.dialect.sql(sql::SESSIONS_OF_DELETE))
                    .bind(account.as_str().to_string())
                    .execute(&mut *conn)
                    .await
                    .map_err(db)?;
            }
            this.account_info(&mut conn, &account).await
        })
    }

    fn reset_password(&self, account: AccountId, password: String) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require_admin(&mut conn).await?;
            if !valid_password(&password) {
                return Err(refused(Refusal::PasswordTooShort));
            }
            this.account_info(&mut conn, &account).await?;
            let hash = this.hash(password).await?;
            let id = account.as_str().to_string();
            sqlx::query(this.dialect.sql(sql::ACCOUNT_SET_PASSWORD))
                .bind(hash)
                .bind(id.clone())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            // Whoever knew the old one is signed out.
            sqlx::query(this.dialect.sql(sql::SESSIONS_OF_DELETE))
                .bind(id)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn change_password(&self, current: String, password: String) -> StoreFuture<()> {
        self.run(|this| async move {
            if !valid_password(&password) {
                return Err(refused(Refusal::PasswordTooShort));
            }
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let row = sqlx::query(this.dialect.sql(sql::ACCOUNT_PASSWORD))
                .bind(this.account())
                .fetch_optional(&mut *conn)
                .await
                .map_err(db)?
                .ok_or_else(|| StoreError::not_found(Entity::Account, &this.account))?;
            let stored = rows::text(&row, 0)?;
            let passwords = this.store.passwords.clone();
            let matches = tokio::task::spawn_blocking(move || passwords.verify(&stored, &current))
                .await
                .map_err(db)?;
            if !matches {
                return Err(refused(Refusal::WrongPassword));
            }
            let hash = this.hash(password).await?;
            sqlx::query(this.dialect.sql(sql::ACCOUNT_SET_PASSWORD))
                .bind(hash)
                .bind(this.account())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn mcp_tokens(&self) -> StoreFuture<Vec<McpToken>> {
        self.run(|this| async move {
            let mut tokens = sqlx::query(this.dialect.sql(sql::MCP_TOKENS_OF))
                .bind(this.account())
                .bind(unix_now())
                .fetch_all(&this.pool)
                .await
                .map_err(db)?
                .iter()
                .map(|row| mcp_token(row, 0))
                .collect::<Result<Vec<_>, _>>()?;
            // Within a second, the ids (ULIDs) are in the order made.
            tokens.sort_by(|a, b| (a.created_at, a.id.as_str()).cmp(&(b.created_at, b.id.as_str())));
            Ok(tokens)
        })
    }

    fn issue_mcp_token(&self, input: NewMcpToken) -> StoreFuture<IssuedMcpToken> {
        self.run(|this| async move {
            let name = input.name.trim().to_string();
            if !valid_token_name(&name) {
                return Err(refused(Refusal::BadTokenName));
            }
            if input.days.is_some_and(|days| !MCP_TOKEN_DAYS.contains(&days)) {
                return Err(refused(Refusal::BadTokenDays));
            }
            // H1: the token and whether it may write, together.
            let mut conn = this.pool.begin().await.map_err(db)?;
            let taken = sqlx::query(this.dialect.sql(sql::MCP_TOKEN_NAME_COUNT))
                .bind(this.account())
                .bind(name.clone())
                .fetch_one(&mut *conn)
                .await
                .map_err(db)?
                .try_get::<i64, _>(0)
                .map_err(db)?;
            if taken > 0 {
                return Err(refused(Refusal::TokenNameTaken));
            }
            let secret = format!("{MCP_TOKEN_PREFIX}{}", new_token()?);
            let id = McpTokenId::new(new_id());
            let created_at = now();
            let expires_at = input.days.map(|days| unix_now() + i64::from(days) * 24 * 60 * 60);
            sqlx::query(this.dialect.sql(sql::MCP_TOKEN_INSERT))
                .bind(id.as_str().to_string())
                .bind(token_hash(&secret))
                .bind(this.account())
                .bind(name.clone())
                .bind(i32::from(input.can_send))
                .bind(created_at.to_rfc3339())
                .bind(expires_at)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            if input.can_write {
                sqlx::query(this.dialect.sql(sql::MCP_TOKEN_WRITE_INSERT))
                    .bind(id.as_str().to_string())
                    .execute(&mut *conn)
                    .await
                    .map_err(db)?;
            }
            conn.commit().await.map_err(db)?;
            Ok(IssuedMcpToken {
                token: McpToken {
                    id,
                    name,
                    can_send: input.can_send,
                    can_write: input.can_write,
                    created_at,
                    expires_at: expires_at.map(Timestamp::from_unix),
                },
                secret,
            })
        })
    }

    fn revoke_mcp_token(&self, id: McpTokenId) -> StoreFuture<()> {
        self.run(|this| async move {
            let revoked = sqlx::query(this.dialect.sql(sql::MCP_TOKEN_DELETE))
                .bind(id.as_str().to_string())
                .bind(this.account())
                .execute(&this.pool)
                .await
                .map_err(db)?
                .rows_affected();
            if revoked == 0 {
                return Err(StoreError::not_found(Entity::McpToken, &id));
            }
            Ok(())
        })
    }
}
