-- The bridge's store in PostgreSQL (`docs/architecture.md` §4.4, §9).
--
-- Ids are text (ULIDs, but taken as given). Times are RFC 3339 text, as the
-- domain writes them. `seq` keeps the order rows were made in, which is the
-- order a project's tree and its variables are read in. JSON (a folder's
-- children, a request, a document) is text: nothing queries inside it.

CREATE TABLE IF NOT EXISTS store_meta (
    name VARCHAR(64) PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(64) NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    admin INTEGER NOT NULL DEFAULT 0,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL
);

-- A login: the hash of the cookie's token, never the token.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash VARCHAR(64) PRIMARY KEY,
    account VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    expires_at BIGINT NOT NULL,
    created_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_account ON sessions (account);

-- A team; `personal_of` names the account whose own space it is.
CREATE TABLE IF NOT EXISTS teams (
    id VARCHAR(64) PRIMARY KEY,
    name TEXT NOT NULL,
    personal_of VARCHAR(64) UNIQUE REFERENCES accounts (id) ON DELETE CASCADE,
    created_at VARCHAR(40) NOT NULL
);

CREATE TABLE IF NOT EXISTS team_members (
    team VARCHAR(64) NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    account VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    role VARCHAR(16) NOT NULL,
    PRIMARY KEY (team, account)
);
CREATE INDEX IF NOT EXISTS team_members_account ON team_members (account);

CREATE TABLE IF NOT EXISTS collections (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    owner_team VARCHAR(64) NOT NULL REFERENCES teams (id),
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    -- Moves with every change to the project's shared content (§7).
    revision BIGINT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS collections_owner_team ON collections (owner_team);

-- A project granted to one account (`account`) or a team's members (`team`).
CREATE TABLE IF NOT EXISTS collection_grants (
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    grantee_kind VARCHAR(8) NOT NULL,
    grantee VARCHAR(64) NOT NULL,
    role VARCHAR(16) NOT NULL,
    PRIMARY KEY (collection, grantee_kind, grantee)
);
CREATE INDEX IF NOT EXISTS collection_grants_grantee ON collection_grants (grantee_kind, grantee);

CREATE TABLE IF NOT EXISTS folders (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    children TEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS folders_collection ON folders (collection);

CREATE TABLE IF NOT EXISTS apis (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    category VARCHAR(16) NOT NULL,
    request TEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS apis_collection ON apis (collection);

CREATE TABLE IF NOT EXISTS top_orders (
    collection VARCHAR(64) PRIMARY KEY REFERENCES collections (id) ON DELETE CASCADE,
    items TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS variables (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    category VARCHAR(32) NOT NULL,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS variables_collection ON variables (collection, category);

-- What each account keeps for itself: `app`, and `workspace/<collection>`.
CREATE TABLE IF NOT EXISTS prefs (
    account VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    scope VARCHAR(160) NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (account, scope)
);
CREATE INDEX IF NOT EXISTS prefs_scope ON prefs (scope);
