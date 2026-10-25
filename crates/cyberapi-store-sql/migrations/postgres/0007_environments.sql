-- A project's named environments (#61), in its order (`place`), and which
-- one a variable is in: a variable without a row is every environment's.
CREATE TABLE IF NOT EXISTS environments (
    id VARCHAR(64) PRIMARY KEY,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    place BIGINT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS environments_collection ON environments (collection);

CREATE TABLE IF NOT EXISTS variable_environments (
    variable VARCHAR(64) PRIMARY KEY REFERENCES variables (id) ON DELETE CASCADE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    environment VARCHAR(64) NOT NULL REFERENCES environments (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS variable_environments_collection ON variable_environments (collection);
