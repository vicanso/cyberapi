//! One account's view of the store (§9.3): what it may see and change.
//!
//! Every call first works out the account's role on the project it names.
//! None at all is "not found" — whether the project exists is not the
//! account's to learn — and too little is "forbidden". A change to a
//! project's tree takes the project's row first ([`sql::COLLECTION_LOCK`]),
//! so two changes cannot plan from the same lists.

use crate::sql::{self, Dialect};
use crate::{SqlStore, db, new_id, now, rows};
use cyberapi_core::access::{CollectionRole, TeamRole, effective_role};
use cyberapi_core::cookie::Cookie;
use cyberapi_core::environments::reordered;
use cyberapi_core::import::{ImportItem, assign_ids};
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Environment, Folder, FolderSettings, HttpRequest, ProjectSettings, Variable,
    VariableCategory,
};
use cyberapi_core::oauth::KeptTokens;
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::secrets::SecretValues;
use cyberapi_core::sync::{ConfigAt, KEPT_VERSIONS, ProjectConfig, ProjectVersion};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{AccountId, CollectionId, EnvironmentId, ItemId, TeamId, Timestamp, VariableId};
use cyberapi_store::rules::{environment_name, known_environment, project_config, required_name};
use cyberapi_store::{
    CollectionEdit, CollectionStore, CookieStore, Entity, HistoryStore, Imported, Inserted, NewCollection, NewVariable,
    PrefStore, StoreError, StoreFuture, TreeStore, VariableEdit, VariableStore, VersionStore, sort_cookies,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::any::AnyRow;
use sqlx::{AnyConnection, AnyPool, Row as _};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;

/// Variables as read from their table, each with its environment (#61).
fn in_environments(variables: Vec<Option<Variable>>, environments: &HashMap<String, EnvironmentId>) -> Vec<Variable> {
    variables
        .into_iter()
        .flatten()
        .map(|variable| Variable {
            environment: environments.get(variable.id.as_str()).cloned(),
            ..variable
        })
        .collect()
}

/// A `settings` column (#66); one that cannot be read is none, as an
/// unreadable request is an empty one.
fn settings_of<T: DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_default()
}

pub(crate) fn workspace_scope(collection: &CollectionId) -> String {
    format!("workspace/{collection}")
}

const APP_SCOPE: &str = "app";
/// C4: the account's OAuth tokens, a document beside its settings.
const OAUTH_SCOPE: &str = "oauth-tokens";

/// D3: the account's values of a project's secret variables.
pub(crate) fn secrets_scope(collection: &CollectionId) -> String {
    format!("secrets/{collection}")
}

#[derive(Clone)]
pub(crate) struct Scoped {
    pub(crate) store: SqlStore,
    pub(crate) pool: AnyPool,
    pub(crate) dialect: Dialect,
    pub(crate) account: AccountId,
}

/// The ways an account reaches a project, as `(kind, role)` rows of
/// [`sql::ROLE_OF`] / [`sql::COLLECTIONS_VISIBLE`], folded into one role.
fn fold_roles<'a>(reached: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<CollectionRole> {
    let mut team: Option<TeamRole> = None;
    let mut grants = Vec::new();
    for (kind, role) in reached {
        if kind == "team" {
            team = team.max(TeamRole::parse(role));
        } else if let Some(role) = CollectionRole::parse(role) {
            grants.push(role);
        }
    }
    effective_role(team, grants)
}

impl Scoped {
    pub(crate) fn new(store: SqlStore, account: AccountId) -> Self {
        Self {
            pool: store.pool.clone(),
            dialect: store.dialect,
            store,
            account,
        }
    }

    pub(crate) fn run<T, F>(&self, work: impl FnOnce(Self) -> F) -> StoreFuture<T>
    where
        F: Future<Output = Result<T, StoreError>> + Send + 'static,
        T: Send + 'static,
    {
        Box::pin(work(self.clone()))
    }

    pub(crate) fn account(&self) -> String {
        self.account.as_str().to_string()
    }

    async fn role_of(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
    ) -> Result<Option<CollectionRole>, StoreError> {
        let (id, account) = (collection.as_str().to_string(), self.account());
        let found = sqlx::query(self.dialect.sql(sql::ROLE_OF))
            .bind(id.clone())
            .bind(account.clone())
            .bind(id.clone())
            .bind(account.clone())
            .bind(id)
            .bind(account)
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?;
        let reached: Vec<(String, String)> = found
            .iter()
            .map(|row| Ok((rows::text(row, 0)?, rows::text(row, 1)?)))
            .collect::<Result<_, StoreError>>()?;
        Ok(fold_roles(
            reached.iter().map(|(kind, role)| (kind.as_str(), role.as_str())),
        ))
    }

    /// The account's role on the project, when it is at least `needed`.
    pub(crate) async fn require(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        needed: CollectionRole,
    ) -> Result<CollectionRole, StoreError> {
        match self.role_of(conn, collection).await? {
            None => Err(StoreError::not_found(Entity::Collection, collection)),
            Some(role) if role < needed => Err(StoreError::Forbidden { needed }),
            Some(role) => Ok(role),
        }
    }

    async fn lock(&self, conn: &mut AnyConnection, collection: &CollectionId) -> Result<(), StoreError> {
        sqlx::query(self.dialect.sql(sql::COLLECTION_LOCK))
            .bind(collection.as_str().to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn touch(&self, conn: &mut AnyConnection, collection: &CollectionId) -> Result<(), StoreError> {
        sqlx::query(self.dialect.sql(sql::COLLECTION_TOUCH))
            .bind(now().to_rfc3339())
            .bind(collection.as_str().to_string())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn get_collection(
        &self,
        conn: &mut AnyConnection,
        id: &CollectionId,
        role: CollectionRole,
    ) -> Result<Collection, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::COLLECTION_GET))
            .bind(id.as_str().to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .ok_or_else(|| StoreError::not_found(Entity::Collection, id))?;
        let mut collection = rows::collection(&row, 0)?;
        collection.role = role;
        Ok(collection)
    }

    /// The project's whole configuration, as a sync reads it.
    async fn config_of(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        name: String,
        description: String,
    ) -> Result<ProjectConfig, StoreError> {
        let tree = self.tree_data(conn, collection).await?;
        let variables = sqlx::query(self.dialect.sql(sql::VARIABLES_ALL_OF))
            .bind(collection.as_str().to_string())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(rows::variable)
            .collect::<Result<Vec<_>, _>>()?;
        let variables = in_environments(variables, &self.variable_environments(conn, collection).await?);
        let environments = self.environments_of(conn, collection).await?;
        Ok(ProjectConfig {
            name,
            description,
            folders: tree.folders,
            apis: tree.apis,
            top_order: tree.top_order,
            variables,
            settings: tree.settings,
            environments,
        })
    }

    /// G4: the project as it is, kept as a version before it is replaced
    /// whole, by this account; the oldest beyond `KEPT_VERSIONS` go. An
    /// empty one — a project made to be uploaded into — is not kept.
    async fn keep_version(&self, conn: &mut AnyConnection, current: &Collection) -> Result<(), StoreError> {
        let collection = &current.id;
        let config = self
            .config_of(conn, collection, current.name.clone(), current.description.clone())
            .await?;
        if config.is_empty() {
            return Ok(());
        }
        let by = sqlx::query(self.dialect.sql(sql::ACCOUNT_INFO))
            .bind(self.account())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .map(|row| rows::text(&row, 1))
            .transpose()?
            .unwrap_or_default();
        sqlx::query(self.dialect.sql(sql::VERSION_INSERT))
            .bind(new_id())
            .bind(collection.as_str().to_string())
            .bind(current.revision as i64)
            .bind(now().to_rfc3339())
            .bind(by)
            .bind(config.folders.len() as i32)
            .bind(config.apis.len() as i32)
            .bind(serde_json::to_string(&config).map_err(db)?)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        let oldest_kept = sqlx::query(self.dialect.sql(sql::VERSION_OLDEST_KEPT))
            .bind(collection.as_str().to_string())
            .bind(KEPT_VERSIONS as i64 - 1)
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
        if let Some(row) = oldest_kept {
            sqlx::query(self.dialect.sql(sql::VERSIONS_TRIM))
                .bind(collection.as_str().to_string())
                .bind(row.try_get::<i64, _>(0).map_err(db)?)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    async fn tree_data(&self, conn: &mut AnyConnection, collection: &CollectionId) -> Result<TreeData, StoreError> {
        let id = collection.as_str().to_string();
        let mut folders: Vec<Folder> = sqlx::query(self.dialect.sql(sql::FOLDERS_OF))
            .bind(id.clone())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(rows::folder)
            .collect::<Result<_, _>>()?;
        // What the folders set (#66), kept beside them.
        let mut settings: HashMap<String, FolderSettings> = sqlx::query(self.dialect.sql(sql::FOLDER_SETTINGS_OF))
            .bind(id.clone())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(|row| Ok((rows::text(row, 0)?, settings_of(&rows::text(row, 1)?))))
            .collect::<Result<_, StoreError>>()?;
        for folder in &mut folders {
            if let Some(own) = settings.remove(folder.id.as_str()) {
                folder.settings = own;
            }
        }
        let project = sqlx::query(self.dialect.sql(sql::PROJECT_SETTINGS_OF))
            .bind(id.clone())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .map(|row| rows::text(&row, 0))
            .transpose()?
            .map(|text| settings_of(&text))
            .unwrap_or_default();
        let apis = sqlx::query(self.dialect.sql(sql::APIS_OF))
            .bind(id.clone())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(rows::api)
            .collect::<Result<_, _>>()?;
        let top_order = match sqlx::query(self.dialect.sql(sql::TOP_ORDER_OF))
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
        {
            Some(row) => serde_json::from_str(&rows::text(&row, 0)?).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(TreeData {
            folders,
            apis,
            top_order,
            settings: project,
        })
    }

    /// A project's named environments (#61), in its order.
    async fn environments_of(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
    ) -> Result<Vec<Environment>, StoreError> {
        sqlx::query(self.dialect.sql(sql::ENVIRONMENTS_OF))
            .bind(collection.as_str().to_string())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(rows::environment)
            .collect()
    }

    /// `id`, if any, is one of `collection`'s environments.
    async fn require_environment(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        id: Option<&EnvironmentId>,
    ) -> Result<(), StoreError> {
        match id {
            Some(id) => known_environment(&self.environments_of(conn, collection).await?, id),
            None => Ok(()),
        }
    }

    /// Which environment each of a project's variables is in, by id.
    async fn variable_environments(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
    ) -> Result<HashMap<String, EnvironmentId>, StoreError> {
        sqlx::query(self.dialect.sql(sql::VARIABLE_ENVIRONMENTS_OF))
            .bind(collection.as_str().to_string())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(|row| Ok((rows::text(row, 0)?, EnvironmentId::new(rows::text(row, 1)?))))
            .collect()
    }

    /// Which environment a variable is in (#61): a row while it is in one.
    async fn write_variable_environment(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        variable: &VariableId,
        environment: Option<&EnvironmentId>,
    ) -> Result<(), StoreError> {
        sqlx::query(self.dialect.sql(sql::VARIABLE_ENVIRONMENT_DELETE))
            .bind(variable.as_str().to_string())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        if let Some(environment) = environment {
            sqlx::query(self.dialect.sql(sql::VARIABLE_ENVIRONMENT_INSERT))
                .bind(variable.as_str().to_string())
                .bind(collection.as_str().to_string())
                .bind(environment.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    /// Writes `environments` as `collection`'s, in their order.
    async fn insert_environments(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        environments: &[Environment],
    ) -> Result<(), StoreError> {
        for (place, environment) in environments.iter().enumerate() {
            sqlx::query(self.dialect.sql(sql::ENVIRONMENT_INSERT))
                .bind(environment.id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .bind(environment.name.clone())
                .bind(i64::try_from(place).unwrap_or(i64::MAX))
                .bind(environment.created_at.to_rfc3339())
                .bind(environment.updated_at.to_rfc3339())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    /// What a folder sets (#66): a row while it sets something, none after.
    async fn write_folder_settings(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        folder: &ItemId,
        settings: &FolderSettings,
    ) -> Result<(), StoreError> {
        if settings.is_empty() {
            sqlx::query(self.dialect.sql(sql::FOLDER_SETTINGS_DELETE))
                .bind(folder.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        } else {
            sqlx::query(self.dialect.pick(&sql::FOLDER_SETTINGS_UPSERT))
                .bind(folder.as_str().to_string())
                .bind(collection.as_str().to_string())
                .bind(serde_json::to_string(settings).map_err(db)?)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    /// What a project sets (#66), as [`Self::write_folder_settings`].
    async fn write_project_settings(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        settings: &ProjectSettings,
    ) -> Result<(), StoreError> {
        if settings.is_empty() {
            sqlx::query(self.dialect.sql(sql::PROJECT_SETTINGS_DELETE))
                .bind(collection.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        } else {
            sqlx::query(self.dialect.pick(&sql::PROJECT_SETTINGS_UPSERT))
                .bind(collection.as_str().to_string())
                .bind(serde_json::to_string(settings).map_err(db)?)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    async fn write_change(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        change: &TreeChange,
    ) -> Result<(), StoreError> {
        for (id, children) in &change.folders {
            sqlx::query(self.dialect.sql(sql::FOLDER_SET_CHILDREN))
                .bind(serde_json::to_string(children).map_err(db)?)
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        if let Some(top_order) = &change.top_order {
            sqlx::query(self.dialect.pick(&sql::TOP_ORDER_UPSERT))
                .bind(collection.as_str().to_string())
                .bind(serde_json::to_string(top_order).map_err(db)?)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    async fn read_doc<T: DeserializeOwned + Default>(
        &self,
        conn: &mut AnyConnection,
        scope: &str,
    ) -> Result<T, StoreError> {
        let row = sqlx::query(self.dialect.sql(sql::PREF_GET))
            .bind(self.account())
            .bind(scope.to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
        Ok(match row {
            Some(row) => serde_json::from_str(&rows::text(&row, 0)?).unwrap_or_default(),
            None => T::default(),
        })
    }

    async fn write_doc(&self, conn: &mut AnyConnection, scope: &str, doc: &impl Serialize) -> Result<(), StoreError> {
        sqlx::query(self.dialect.pick(&sql::PREF_UPSERT))
            .bind(self.account())
            .bind(scope.to_string())
            .bind(serde_json::to_string(doc).map_err(db)?)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        Ok(())
    }

    /// Creates a folder or an API: the shared steps around the insert.
    async fn insert_item<T>(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        insert: impl AsyncFnOnce(&mut AnyConnection, ItemId) -> Result<T, StoreError>,
    ) -> Result<Inserted<T>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.require(&mut tx, &collection, CollectionRole::Editor).await?;
        self.lock(&mut tx, &collection).await?;
        let id = ItemId::new(new_id());
        let change = self
            .tree_data(&mut tx, &collection)
            .await?
            .plan_insert(&id, parent.as_ref())?;
        let item = insert(&mut tx, id).await?;
        self.write_change(&mut tx, &collection, &change).await?;
        self.touch(&mut tx, &collection).await?;
        tx.commit().await.map_err(db)?;
        Ok(Inserted { item, change })
    }
}

impl Scoped {
    /// The projects the reader can see, each with the reader's role, in the
    /// order they were made.
    async fn visible(&self) -> Result<Vec<Collection>, StoreError> {
        let account = self.account();
        let found = sqlx::query(self.dialect.sql(sql::COLLECTIONS_VISIBLE))
            .bind(account.clone())
            .bind(account.clone())
            .bind(account)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        // One row per way the project is reached; fold them, keeping the
        // order projects were made in.
        let mut order: Vec<CollectionId> = Vec::new();
        let mut reached: HashMap<CollectionId, (Collection, Vec<(String, String)>)> = HashMap::new();
        for row in &found {
            let collection = rows::collection(row, 1)?;
            let way = (rows::text(row, 8)?, rows::text(row, 9)?);
            let entry = reached.entry(collection.id.clone()).or_insert_with(|| {
                order.push(collection.id.clone());
                (collection, Vec::new())
            });
            entry.1.push(way);
        }
        Ok(order
            .into_iter()
            .filter_map(|id| reached.remove(&id))
            .filter_map(|(mut collection, ways)| {
                collection.role = fold_roles(ways.iter().map(|(k, r)| (k.as_str(), r.as_str())))?;
                Some(collection)
            })
            .collect())
    }
}

impl CollectionStore for Scoped {
    fn list(&self) -> StoreFuture<Vec<Collection>> {
        self.run(|this| async move { this.visible().await })
    }

    fn api_counts(&self) -> StoreFuture<BTreeMap<CollectionId, usize>> {
        self.run(|this| async move {
            let visible: HashSet<CollectionId> = this.visible().await?.into_iter().map(|c| c.id).collect();
            let found = sqlx::query(this.dialect.sql(sql::API_COUNTS))
                .fetch_all(&this.pool)
                .await
                .map_err(db)?;
            let mut counts = BTreeMap::new();
            for row in &found {
                let id = CollectionId::new(rows::text(row, 0)?);
                let count = usize::try_from(row.try_get::<i64, _>(1).map_err(db)?).unwrap_or_default();
                if visible.contains(&id) {
                    counts.insert(id, count);
                }
            }
            Ok(counts)
        })
    }

    fn get(&self, id: CollectionId) -> StoreFuture<Collection> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let role = this.require(&mut conn, &id, CollectionRole::Viewer).await?;
            this.get_collection(&mut conn, &id, role).await
        })
    }

    /// In the given team, which the account must be in, or else in the
    /// account's own space.
    fn create(&self, input: NewCollection) -> StoreFuture<Collection> {
        self.run(|this| async move {
            let name = required_name(&input.name)?;
            let mut tx = this.pool.begin().await.map_err(db)?;
            let team = match input.owner {
                Some(team) => team,
                None => {
                    let row = sqlx::query(this.dialect.sql(sql::PERSONAL_TEAM))
                        .bind(this.account())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(db)?
                        .ok_or_else(|| StoreError::not_found(Entity::Team, "personal space"))?;
                    TeamId::new(rows::text(&row, 0)?)
                }
            };
            let team_role = sqlx::query(this.dialect.sql(sql::TEAM_ROLE))
                .bind(team.as_str().to_string())
                .bind(this.account())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
                .map(|row| rows::text(&row, 0))
                .transpose()?
                .and_then(|role| TeamRole::parse(&role))
                .ok_or_else(|| StoreError::not_found(Entity::Team, &team))?;
            // G1: an id given is kept, unless a project has it — any
            // project, seen by this account or not.
            if let Some(id) = &input.id {
                let taken = sqlx::query(this.dialect.sql(sql::COLLECTION_EXISTS))
                    .bind(id.as_str().to_string())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(db)?;
                if taken.is_some() {
                    return Err(StoreError::IdTaken { id: id.to_string() });
                }
            }
            let at = now();
            let collection = Collection {
                id: input.id.unwrap_or_else(|| CollectionId::new(new_id())),
                name,
                description: input.description,
                created_at: at,
                updated_at: at,
                revision: 0,
                owner: Some(team.clone()),
                role: team_role.on_team_projects(),
            };
            sqlx::query(this.dialect.sql(sql::COLLECTION_INSERT))
                .bind(collection.id.as_str().to_string())
                .bind(team.as_str().to_string())
                .bind(collection.name.clone())
                .bind(collection.description.clone())
                .bind(at.to_rfc3339())
                .bind(at.to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;
            Ok(collection)
        })
    }

    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection> {
        self.run(|this| async move {
            let name = required_name(&edit.name)?;
            let mut tx = this.pool.begin().await.map_err(db)?;
            let role = this.require(&mut tx, &id, CollectionRole::Manager).await?;
            sqlx::query(this.dialect.sql(sql::COLLECTION_UPDATE))
                .bind(name)
                .bind(edit.description)
                .bind(now().to_rfc3339())
                .bind(id.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            let collection = this.get_collection(&mut tx, &id, role).await?;
            tx.commit().await.map_err(db)?;
            Ok(collection)
        })
    }

    /// The database takes the project's tree, variables and grants with it;
    /// every account's workspace state goes here.
    fn delete(&self, id: CollectionId) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &id, CollectionRole::Manager).await?;
            for scope in [workspace_scope(&id), secrets_scope(&id)] {
                sqlx::query(this.dialect.sql(sql::PREFS_DELETE_SCOPE))
                    .bind(scope)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            sqlx::query(this.dialect.sql(sql::COLLECTION_DELETE))
                .bind(id.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }
}

impl TreeStore for Scoped {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.tree_data(&mut conn, &collection).await
        })
    }

    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.run(|this| async move {
            let name = required_name(&name)?;
            let dialect = this.dialect;
            let owner = collection.clone();
            this.insert_item(collection, parent, async move |conn: &mut AnyConnection, id: ItemId| {
                let at = now();
                sqlx::query(dialect.sql(sql::FOLDER_INSERT))
                    .bind(id.as_str().to_string())
                    .bind(owner.as_str().to_string())
                    .bind(name.clone())
                    .bind("[]")
                    .bind(at.to_rfc3339())
                    .bind(at.to_rfc3339())
                    .execute(&mut *conn)
                    .await
                    .map_err(db)?;
                Ok(Folder {
                    id,
                    collection: owner,
                    name,
                    children: Vec::new(),
                    settings: Default::default(),
                    created_at: at,
                    updated_at: at,
                })
            })
            .await
        })
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.run(|this| async move {
            let name = required_name(&name)?;
            let dialect = this.dialect;
            let owner = collection.clone();
            this.insert_item(collection, parent, async move |conn: &mut AnyConnection, id: ItemId| {
                let at = now();
                let request = HttpRequest::default();
                sqlx::query(dialect.sql(sql::API_INSERT))
                    .bind(id.as_str().to_string())
                    .bind(owner.as_str().to_string())
                    .bind(name.clone())
                    .bind("http")
                    .bind(request.to_json())
                    .bind(at.to_rfc3339())
                    .bind(at.to_rfc3339())
                    .execute(&mut *conn)
                    .await
                    .map_err(db)?;
                Ok(ApiSetting {
                    id,
                    collection: owner,
                    name,
                    category: ApiCategory::Http,
                    request,
                    created_at: at,
                    updated_at: at,
                })
            })
            .await
        })
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.run(|this| async move {
            let name = required_name(&name)?;
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            let at = now().to_rfc3339();
            let mut renamed = 0;
            for statement in [sql::FOLDER_RENAME, sql::API_RENAME] {
                renamed += sqlx::query(this.dialect.sql(statement))
                    .bind(name.clone())
                    .bind(at.clone())
                    .bind(item.as_str().to_string())
                    .bind(collection.as_str().to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?
                    .rows_affected();
            }
            if renamed == 0 {
                return Err(StoreError::not_found(Entity::Item, &item));
            }
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)
        })
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            let saved = sqlx::query(this.dialect.sql(sql::API_SAVE))
                .bind(request.to_json())
                .bind(now().to_rfc3339())
                .bind(api.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            if saved.rows_affected() == 0 {
                return Err(StoreError::not_found(Entity::Api, &api));
            }
            let row = sqlx::query(this.dialect.sql(sql::API_GET))
                .bind(api.as_str().to_string())
                .bind(collection.as_str().to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            let found = rows::api(&row)?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(found)
        })
    }

    fn set_folder_settings(
        &self,
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    ) -> StoreFuture<Folder> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            let touched = sqlx::query(this.dialect.sql(sql::FOLDER_TOUCH))
                .bind(now().to_rfc3339())
                .bind(folder.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            if touched.rows_affected() == 0 {
                return Err(StoreError::not_found(Entity::Item, &folder));
            }
            this.write_folder_settings(&mut tx, &collection, &folder, &settings)
                .await?;
            let row = sqlx::query(this.dialect.sql(sql::FOLDER_GET))
                .bind(folder.as_str().to_string())
                .bind(collection.as_str().to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            let found = rows::folder(&row)?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(Folder { settings, ..found })
        })
    }

    fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.write_project_settings(&mut tx, &collection, &settings).await?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)
        })
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let change = this.tree_data(&mut tx, &collection).await?.plan_move(&item, &to)?;
            this.write_change(&mut tx, &collection, &change).await?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(change)
        })
    }

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let assigned = assign_ids(&items, &mut || ItemId::new(new_id()));
            let change = this
                .tree_data(&mut tx, &collection)
                .await?
                .plan_import(&assigned.top, parent.as_ref())?;
            let at = now();
            let mut imported = Imported {
                change,
                ..Default::default()
            };
            for (id, name, children, settings) in assigned.folders {
                sqlx::query(this.dialect.sql(sql::FOLDER_INSERT))
                    .bind(id.as_str().to_string())
                    .bind(collection.as_str().to_string())
                    .bind(name.clone())
                    .bind(serde_json::to_string(&children).map_err(db)?)
                    .bind(at.to_rfc3339())
                    .bind(at.to_rfc3339())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                this.write_folder_settings(&mut tx, &collection, &id, &settings).await?;
                imported.folders.push(Folder {
                    id,
                    collection: collection.clone(),
                    name,
                    children,
                    settings,
                    created_at: at,
                    updated_at: at,
                });
            }
            for (id, name, request) in assigned.apis {
                sqlx::query(this.dialect.sql(sql::API_INSERT))
                    .bind(id.as_str().to_string())
                    .bind(collection.as_str().to_string())
                    .bind(name.clone())
                    .bind("http")
                    .bind(request.to_json())
                    .bind(at.to_rfc3339())
                    .bind(at.to_rfc3339())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                imported.apis.push(ApiSetting {
                    id,
                    collection: collection.clone(),
                    name,
                    category: ApiCategory::Http,
                    request,
                    created_at: at,
                    updated_at: at,
                });
            }
            this.write_change(&mut tx, &collection, &imported.change).await?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(imported)
        })
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let removal = this.tree_data(&mut tx, &collection).await?.plan_remove(&item)?;
            for (statement, ids) in [(sql::FOLDER_DELETE, &removal.folders), (sql::API_DELETE, &removal.apis)] {
                for id in ids {
                    sqlx::query(this.dialect.sql(statement))
                        .bind(id.as_str().to_string())
                        .bind(collection.as_str().to_string())
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                }
            }
            this.write_change(&mut tx, &collection, &removal.change).await?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(removal)
        })
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            let role = this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            let found = this.get_collection(&mut conn, &collection, role).await?;
            let config = this
                .config_of(&mut conn, &collection, found.name, found.description)
                .await?;
            Ok(ConfigAt {
                revision: found.revision,
                config,
            })
        })
    }

    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            let role = this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let current = this.get_collection(&mut tx, &collection, role).await?;
            let mut config = project_config(&collection, config)?;
            if role < CollectionRole::Manager {
                // An editor's sync carries the tree and the variables; the
                // name and description stay the managers'.
                config.name = current.name.clone();
                config.description = current.description.clone();
            }
            if let Some(base) = base
                && base != current.revision
            {
                return Err(StoreError::Stale {
                    base,
                    revision: current.revision,
                });
            }
            // G4: what it was, kept before it is replaced.
            this.keep_version(&mut tx, &current).await?;
            let id = collection.as_str().to_string();
            let items = config.folders.iter().map(|folder| folder.id.as_str());
            let items = items.chain(config.apis.iter().map(|api| api.id.as_str()));
            let owned = items
                .map(|item| (item, [sql::FOLDER_OWNER, sql::API_OWNER].as_slice()))
                .chain(
                    config
                        .variables
                        .iter()
                        .map(|variable| (variable.id.as_str(), [sql::VARIABLE_OWNER].as_slice())),
                )
                .chain(
                    config
                        .environments
                        .iter()
                        .map(|environment| (environment.id.as_str(), [sql::ENVIRONMENT_OWNER].as_slice())),
                );
            for (item, statements) in owned {
                for statement in statements {
                    let owner = sqlx::query(this.dialect.sql(statement))
                        .bind(item.to_string())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(db)?
                        .map(|row| rows::text(&row, 0))
                        .transpose()?;
                    if owner.is_some_and(|owner| owner != id) {
                        return Err(StoreError::IdTaken { id: item.to_string() });
                    }
                }
            }
            let had: HashSet<String> = sqlx::query(this.dialect.sql(sql::API_IDS_OF))
                .bind(id.clone())
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?
                .iter()
                .map(|row| rows::text(row, 0))
                .collect::<Result<_, _>>()?;
            let staying: HashSet<&str> = config.apis.iter().map(|api| api.id.as_str()).collect();
            // An API that goes takes its responses with it.
            for gone in had.iter().filter(|api| !staying.contains(api.as_str())) {
                sqlx::query(this.dialect.sql(sql::API_DELETE))
                    .bind(gone.clone())
                    .bind(id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            // A variable's environment row goes with it (ON DELETE CASCADE).
            for statement in [
                sql::FOLDERS_DELETE_OF,
                sql::VARIABLES_DELETE_OF,
                sql::ENVIRONMENTS_DELETE_OF,
            ] {
                sqlx::query(this.dialect.sql(statement))
                    .bind(id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            for folder in &config.folders {
                sqlx::query(this.dialect.sql(sql::FOLDER_INSERT))
                    .bind(folder.id.as_str().to_string())
                    .bind(id.clone())
                    .bind(folder.name.clone())
                    .bind(serde_json::to_string(&folder.children).map_err(db)?)
                    .bind(folder.created_at.to_rfc3339())
                    .bind(folder.updated_at.to_rfc3339())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                // The rows of before went with their folders.
                this.write_folder_settings(&mut tx, &collection, &folder.id, &folder.settings)
                    .await?;
            }
            this.write_project_settings(&mut tx, &collection, &config.settings)
                .await?;
            for api in &config.apis {
                if had.contains(api.id.as_str()) {
                    sqlx::query(this.dialect.sql(sql::API_REWRITE))
                        .bind(api.name.clone())
                        .bind("http")
                        .bind(api.request.to_json())
                        .bind(api.created_at.to_rfc3339())
                        .bind(api.updated_at.to_rfc3339())
                        .bind(api.id.as_str().to_string())
                        .bind(id.clone())
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                } else {
                    sqlx::query(this.dialect.sql(sql::API_INSERT))
                        .bind(api.id.as_str().to_string())
                        .bind(id.clone())
                        .bind(api.name.clone())
                        .bind("http")
                        .bind(api.request.to_json())
                        .bind(api.created_at.to_rfc3339())
                        .bind(api.updated_at.to_rfc3339())
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                }
            }
            this.insert_environments(&mut tx, &collection, &config.environments)
                .await?;
            for variable in &config.variables {
                sqlx::query(this.dialect.sql(sql::VARIABLE_INSERT))
                    .bind(variable.id.as_str().to_string())
                    .bind(id.clone())
                    .bind(variable.category.as_str())
                    .bind(variable.name.clone())
                    .bind(variable.value.clone())
                    .bind(i32::from(variable.enabled))
                    .bind(variable.created_at.to_rfc3339())
                    .bind(variable.updated_at.to_rfc3339())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                this.write_variable_environment(&mut tx, &collection, &variable.id, variable.environment.as_ref())
                    .await?;
            }
            let top = TreeChange {
                top_order: Some(config.top_order.clone()),
                ..Default::default()
            };
            this.write_change(&mut tx, &collection, &top).await?;
            sqlx::query(this.dialect.sql(sql::COLLECTION_UPDATE))
                .bind(config.name)
                .bind(config.description)
                .bind(now().to_rfc3339())
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;
            Ok(current.revision + 1)
        })
    }
}

impl VariableStore for Scoped {
    /// Sorted here, not by the database: its collation (PostgreSQL's is the
    /// locale's) would order names differently from every other backend.
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            let found = sqlx::query(this.dialect.sql(sql::VARIABLES_OF))
                .bind(collection.as_str().to_string())
                .bind(category.as_str())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?;
            let found = found.iter().map(rows::variable).collect::<Result<Vec<_>, _>>()?;
            let mut variables = in_environments(found, &this.variable_environments(&mut conn, &collection).await?);
            variables.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(variables)
        })
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &input.collection, CollectionRole::Editor).await?;
            this.require_environment(&mut tx, &input.collection, input.environment.as_ref())
                .await?;
            let at = now();
            let variable = Variable {
                id: VariableId::new(new_id()),
                collection: input.collection,
                category: input.category,
                name: input.name,
                value: input.value,
                enabled: input.enabled,
                environment: input.environment,
                created_at: at,
                updated_at: at,
            };
            sqlx::query(this.dialect.sql(sql::VARIABLE_INSERT))
                .bind(variable.id.as_str().to_string())
                .bind(variable.collection.as_str().to_string())
                .bind(variable.category.as_str())
                .bind(variable.name.clone())
                .bind(variable.value.clone())
                .bind(i32::from(variable.enabled))
                .bind(at.to_rfc3339())
                .bind(at.to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            this.write_variable_environment(
                &mut tx,
                &variable.collection,
                &variable.id,
                variable.environment.as_ref(),
            )
            .await?;
            this.touch(&mut tx, &variable.collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(variable)
        })
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.require_environment(&mut tx, &collection, edit.environment.as_ref())
                .await?;
            let updated = sqlx::query(this.dialect.sql(sql::VARIABLE_UPDATE))
                .bind(edit.name)
                .bind(edit.value)
                .bind(i32::from(edit.enabled))
                .bind(now().to_rfc3339())
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            if updated.rows_affected() == 0 {
                return Err(StoreError::not_found(Entity::Variable, &id));
            }
            let row = sqlx::query(this.dialect.sql(sql::VARIABLE_GET))
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            let variable = rows::variable(&row)?.ok_or_else(|| StoreError::not_found(Entity::Variable, &id))?;
            this.write_variable_environment(&mut tx, &collection, &id, edit.environment.as_ref())
                .await?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(Variable {
                environment: edit.environment,
                ..variable
            })
        })
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            for id in &ids {
                sqlx::query(this.dialect.sql(sql::VARIABLE_DELETE))
                    .bind(id.as_str().to_string())
                    .bind(collection.as_str().to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)
        })
    }

    fn environments(&self, collection: CollectionId) -> StoreFuture<Vec<Environment>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.environments_of(&mut conn, &collection).await
        })
    }

    fn create_environment(&self, collection: CollectionId, name: String) -> StoreFuture<Environment> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let existing = this.environments_of(&mut tx, &collection).await?;
            let at = now();
            let environment = Environment {
                id: EnvironmentId::new(new_id()),
                collection: collection.clone(),
                name: environment_name(&existing, &name, None)?,
                created_at: at,
                updated_at: at,
            };
            sqlx::query(this.dialect.sql(sql::ENVIRONMENT_INSERT))
                .bind(environment.id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .bind(environment.name.clone())
                .bind(i64::try_from(existing.len()).unwrap_or(i64::MAX))
                .bind(at.to_rfc3339())
                .bind(at.to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(environment)
        })
    }

    fn rename_environment(
        &self,
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    ) -> StoreFuture<Environment> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let existing = this.environments_of(&mut tx, &collection).await?;
            let name = environment_name(&existing, &name, Some(&id))?;
            let found = existing
                .into_iter()
                .find(|environment| environment.id == id)
                .ok_or_else(|| StoreError::not_found(Entity::Environment, &id))?;
            let at = now();
            sqlx::query(this.dialect.sql(sql::ENVIRONMENT_RENAME))
                .bind(name.clone())
                .bind(at.to_rfc3339())
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(Environment {
                name,
                updated_at: at,
                ..found
            })
        })
    }

    fn delete_environment(&self, collection: CollectionId, id: EnvironmentId) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.require_environment(&mut tx, &collection, Some(&id)).await?;
            // Its variables first, by id: their rows go with them.
            let inside: Vec<String> = sqlx::query(this.dialect.sql(sql::ENVIRONMENT_VARIABLES))
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?
                .iter()
                .map(|row| rows::text(row, 0))
                .collect::<Result<_, _>>()?;
            for variable in inside {
                sqlx::query(this.dialect.sql(sql::VARIABLE_DELETE))
                    .bind(variable)
                    .bind(collection.as_str().to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            sqlx::query(this.dialect.sql(sql::ENVIRONMENT_DELETE))
                .bind(id.as_str().to_string())
                .bind(collection.as_str().to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)
        })
    }

    fn reorder_environments(
        &self,
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    ) -> StoreFuture<Vec<Environment>> {
        self.run(|this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require(&mut tx, &collection, CollectionRole::Editor).await?;
            this.lock(&mut tx, &collection).await?;
            let placed = reordered(&this.environments_of(&mut tx, &collection).await?, &order);
            for (place, environment) in placed.iter().enumerate() {
                sqlx::query(this.dialect.sql(sql::ENVIRONMENT_PLACE))
                    .bind(i64::try_from(place).unwrap_or(i64::MAX))
                    .bind(environment.id.as_str().to_string())
                    .bind(collection.as_str().to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            this.touch(&mut tx, &collection).await?;
            tx.commit().await.map_err(db)?;
            Ok(placed)
        })
    }
}

impl PrefStore for Scoped {
    fn app(&self) -> StoreFuture<AppPrefs> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.read_doc(&mut conn, APP_SCOPE).await
        })
    }

    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.write_doc(&mut conn, APP_SCOPE, &prefs).await
        })
    }

    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.read_doc(&mut conn, &workspace_scope(&collection)).await
        })
    }

    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.write_doc(&mut conn, &workspace_scope(&collection), &prefs).await
        })
    }

    fn clear_workspaces(&self) -> StoreFuture<()> {
        self.run(|this| async move {
            sqlx::query(this.dialect.sql(sql::PREFS_CLEAR_WORKSPACES))
                .bind(this.account())
                .execute(&this.pool)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn oauth_tokens(&self) -> StoreFuture<KeptTokens> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.read_doc(&mut conn, OAUTH_SCOPE).await
        })
    }

    fn set_oauth_tokens(&self, tokens: KeptTokens) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.write_doc(&mut conn, OAUTH_SCOPE, &tokens).await
        })
    }

    /// A viewer has values of their own too.
    fn secret_values(&self, collection: CollectionId) -> StoreFuture<SecretValues> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.read_doc(&mut conn, &secrets_scope(&collection)).await
        })
    }

    fn set_secret_values(&self, collection: CollectionId, values: SecretValues) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            this.write_doc(&mut conn, &secrets_scope(&collection), &values).await
        })
    }
}

impl Scoped {
    /// The project's API `api`, as a history's owner: any reader of the
    /// project keeps their own.
    async fn require_api(
        &self,
        conn: &mut AnyConnection,
        collection: &CollectionId,
        api: &ItemId,
    ) -> Result<(), StoreError> {
        self.require(conn, collection, CollectionRole::Viewer).await?;
        sqlx::query(self.dialect.sql(sql::API_GET))
            .bind(api.as_str().to_string())
            .bind(collection.as_str().to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .map(|_| ())
            .ok_or_else(|| StoreError::not_found(Entity::Api, api))
    }

    async fn responses(
        &self,
        conn: &mut AnyConnection,
        statement: &'static str,
        collection: &CollectionId,
        api: &ItemId,
    ) -> Result<Vec<HttpResponse>, StoreError> {
        sqlx::query(self.dialect.sql(statement))
            .bind(self.account())
            .bind(collection.as_str().to_string())
            .bind(api.as_str().to_string())
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(response)
            .collect()
    }
}

/// A kept response: the whole entry is its document.
fn response(row: &AnyRow) -> Result<HttpResponse, StoreError> {
    serde_json::from_str(&rows::text(row, 0)?).map_err(db)
}

impl HistoryStore for Scoped {
    fn add(&self, response: HttpResponse, keep: usize) -> StoreFuture<HttpResponse> {
        self.run(move |this| async move {
            let mut tx = this.pool.begin().await.map_err(db)?;
            this.require_api(&mut tx, &response.collection, &response.api).await?;
            let kept = HttpResponse {
                id: new_id(),
                created_at: now(),
                ..response
            };
            let (collection, api) = (kept.collection.as_str().to_string(), kept.api.as_str().to_string());
            sqlx::query(this.dialect.sql(sql::RESPONSE_INSERT))
                .bind(kept.id.clone())
                .bind(this.account())
                .bind(collection.clone())
                .bind(api.clone())
                .bind(serde_json::to_string(&kept).map_err(db)?)
                .bind(kept.created_at.to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            // The oldest beyond the newest `keep` go.
            let oldest_kept = sqlx::query(this.dialect.sql(sql::RESPONSE_OLDEST_KEPT))
                .bind(this.account())
                .bind(collection.clone())
                .bind(api.clone())
                .bind(keep.max(1) as i64 - 1)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
            if let Some(row) = oldest_kept {
                sqlx::query(this.dialect.sql(sql::RESPONSES_TRIM))
                    .bind(this.account())
                    .bind(collection)
                    .bind(api)
                    .bind(row.try_get::<i64, _>(0).map_err(db)?)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
            tx.commit().await.map_err(db)?;
            Ok(kept)
        })
    }

    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require_api(&mut conn, &collection, &api).await?;
            this.responses(&mut conn, sql::RESPONSES_OF, &collection, &api).await
        })
    }

    fn recent(&self, collection: CollectionId, limit: usize) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            sqlx::query(this.dialect.sql(sql::RESPONSES_RECENT))
                .bind(this.account())
                .bind(collection.as_str().to_string())
                .bind(limit as i64)
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?
                .iter()
                .map(|row| response(row).map(HttpResponse::summary))
                .collect()
        })
    }

    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            let mut found = Vec::new();
            for api in &apis {
                found.extend(
                    this.responses(&mut conn, sql::RESPONSE_NEWEST, &collection, api)
                        .await?,
                );
            }
            Ok(found)
        })
    }

    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require_api(&mut conn, &collection, &api).await?;
            sqlx::query(this.dialect.sql(sql::RESPONSES_CLEAR))
                .bind(this.account())
                .bind(collection.as_str().to_string())
                .bind(api.as_str().to_string())
                .execute(&mut *conn)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    /// The account's own, of every project; nobody else's.
    fn clear_all(&self) -> StoreFuture<()> {
        self.run(|this| async move {
            sqlx::query(this.dialect.sql(sql::RESPONSES_CLEAR_ALL))
                .bind(this.account())
                .execute(&this.pool)
                .await
                .map_err(db)?;
            Ok(())
        })
    }
}

/// G4: a project's versions; a viewer reads them.
impl VersionStore for Scoped {
    fn versions(&self, collection: CollectionId) -> StoreFuture<Vec<ProjectVersion>> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            sqlx::query(this.dialect.sql(sql::VERSIONS_OF))
                .bind(collection.as_str().to_string())
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?
                .iter()
                .map(|row| {
                    Ok(ProjectVersion {
                        id: rows::text(row, 0)?,
                        revision: row.try_get::<i64, _>(1).map_err(db)?.max(0) as u64,
                        saved_at: Timestamp::parse_lenient(&rows::text(row, 2)?),
                        by: rows::text(row, 3)?,
                        folders: row.try_get::<i32, _>(4).map_err(db)?.max(0) as u32,
                        apis: row.try_get::<i32, _>(5).map_err(db)?.max(0) as u32,
                    })
                })
                .collect()
        })
    }

    fn version(&self, collection: CollectionId, id: String) -> StoreFuture<ProjectConfig> {
        self.run(|this| async move {
            let mut conn = this.pool.acquire().await.map_err(db)?;
            this.require(&mut conn, &collection, CollectionRole::Viewer).await?;
            let row = sqlx::query(this.dialect.sql(sql::VERSION_DOC))
                .bind(collection.as_str().to_string())
                .bind(id.clone())
                .fetch_optional(&mut *conn)
                .await
                .map_err(db)?
                .ok_or_else(|| StoreError::not_found(Entity::Collection, &id))?;
            serde_json::from_str(&rows::text(&row, 0)?).map_err(db)
        })
    }
}

/// The key a cookie is kept under: its domain, path and name, hashed.
fn cookie_key(domain: &str, path: &str, name: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(format!("{domain}\t{path}\t{name}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The account's own: no project, so no role to work out.
impl CookieStore for Scoped {
    fn list(&self) -> StoreFuture<Vec<Cookie>> {
        self.run(|this| async move {
            let found = sqlx::query(this.dialect.sql(sql::COOKIES_OF))
                .bind(this.account())
                .fetch_all(&this.pool)
                .await
                .map_err(db)?;
            let mut cookies: Vec<Cookie> = found
                .iter()
                .map(|row| serde_json::from_str(&rows::text(row, 0)?).map_err(db))
                .collect::<Result<_, _>>()?;
            sort_cookies(&mut cookies);
            Ok(cookies)
        })
    }

    fn put(&self, cookies: Vec<Cookie>) -> StoreFuture<()> {
        self.run(|this| async move {
            let at = now();
            let mut tx = this.pool.begin().await.map_err(db)?;
            for cookie in cookies {
                let key = cookie_key(&cookie.domain, &cookie.path, &cookie.name);
                if cookie.is_expired(at) {
                    sqlx::query(this.dialect.sql(sql::COOKIE_DELETE))
                        .bind(this.account())
                        .bind(key)
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                } else {
                    sqlx::query(this.dialect.pick(&sql::COOKIE_UPSERT))
                        .bind(this.account())
                        .bind(key)
                        .bind(serde_json::to_string(&cookie).map_err(db)?)
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                }
            }
            tx.commit().await.map_err(db)
        })
    }

    fn delete(&self, domain: String, path: String, name: String) -> StoreFuture<()> {
        self.run(|this| async move {
            sqlx::query(this.dialect.sql(sql::COOKIE_DELETE))
                .bind(this.account())
                .bind(cookie_key(&domain, &path, &name))
                .execute(&this.pool)
                .await
                .map_err(db)?;
            Ok(())
        })
    }

    fn clear(&self) -> StoreFuture<()> {
        self.run(|this| async move {
            sqlx::query(this.dialect.sql(sql::COOKIES_CLEAR))
                .bind(this.account())
                .execute(&this.pool)
                .await
                .map_err(db)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ways_in_fold_to_the_strongest() {
        assert_eq!(fold_roles([]), None);
        assert_eq!(fold_roles([("team", "member")]), Some(CollectionRole::Editor));
        assert_eq!(
            fold_roles([("team", "member"), ("grant", "manager")]),
            Some(CollectionRole::Manager)
        );
        assert_eq!(
            fold_roles([("grant", "viewer"), ("grant", "bogus")]),
            Some(CollectionRole::Viewer)
        );
    }
}
