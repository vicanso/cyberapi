-- The bridge's store in MySQL 8.0.19 and later (`docs/architecture.md` §4.4,
-- §9). The PostgreSQL file's tables, column for column.
--
-- Every table is utf8mb4 with the binary collation: MySQL's default ignores
-- case and accents, which would make `Alice` and `alice` one account. JSON
-- and long text are MEDIUMTEXT: TEXT stops at 64 KiB, which a request body
-- passes. Indexes are declared in the tables: MySQL has no
-- `CREATE INDEX IF NOT EXISTS`, and every statement here must run again
-- safely, since MySQL runs DDL outside any transaction.

CREATE TABLE IF NOT EXISTS store_meta (
    name VARCHAR(64) NOT NULL PRIMARY KEY,
    value MEDIUMTEXT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS accounts (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    name VARCHAR(64) NOT NULL UNIQUE,
    display_name MEDIUMTEXT NOT NULL,
    password_hash MEDIUMTEXT NOT NULL,
    admin INT NOT NULL DEFAULT 0,
    disabled INT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS sessions (
    token_hash VARCHAR(64) NOT NULL PRIMARY KEY,
    account VARCHAR(64) NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    KEY sessions_account (account),
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS teams (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    name MEDIUMTEXT NOT NULL,
    personal_of VARCHAR(64) NULL UNIQUE,
    created_at VARCHAR(40) NOT NULL,
    FOREIGN KEY (personal_of) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS team_members (
    team VARCHAR(64) NOT NULL,
    account VARCHAR(64) NOT NULL,
    role VARCHAR(16) NOT NULL,
    PRIMARY KEY (team, account),
    KEY team_members_account (account),
    FOREIGN KEY (team) REFERENCES teams (id) ON DELETE CASCADE,
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS collections (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    owner_team VARCHAR(64) NOT NULL,
    name MEDIUMTEXT NOT NULL,
    description MEDIUMTEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    KEY collections_owner_team (owner_team),
    FOREIGN KEY (owner_team) REFERENCES teams (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS collection_grants (
    collection VARCHAR(64) NOT NULL,
    grantee_kind VARCHAR(8) NOT NULL,
    grantee VARCHAR(64) NOT NULL,
    role VARCHAR(16) NOT NULL,
    PRIMARY KEY (collection, grantee_kind, grantee),
    KEY collection_grants_grantee (grantee_kind, grantee),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS folders (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL,
    name MEDIUMTEXT NOT NULL,
    children MEDIUMTEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    KEY folders_collection (collection),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS apis (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL,
    name MEDIUMTEXT NOT NULL,
    category VARCHAR(16) NOT NULL,
    request MEDIUMTEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    KEY apis_collection (collection),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS top_orders (
    collection VARCHAR(64) NOT NULL PRIMARY KEY,
    items MEDIUMTEXT NOT NULL,
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS variables (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL,
    category VARCHAR(32) NOT NULL,
    name MEDIUMTEXT NOT NULL,
    value MEDIUMTEXT NOT NULL,
    enabled INT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    KEY variables_collection (collection, category),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS prefs (
    account VARCHAR(64) NOT NULL,
    scope VARCHAR(160) NOT NULL,
    doc MEDIUMTEXT NOT NULL,
    PRIMARY KEY (account, scope),
    KEY prefs_scope (scope),
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
