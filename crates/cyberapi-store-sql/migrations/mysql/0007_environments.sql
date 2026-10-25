-- A project's named environments (#61), in its order (`place`), and which
-- one a variable is in: a variable without a row is every environment's.
CREATE TABLE IF NOT EXISTS environments (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    collection VARCHAR(64) NOT NULL,
    name MEDIUMTEXT NOT NULL,
    place BIGINT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    KEY environments_collection (collection),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS variable_environments (
    variable VARCHAR(64) NOT NULL PRIMARY KEY,
    collection VARCHAR(64) NOT NULL,
    environment VARCHAR(64) NOT NULL,
    KEY variable_environments_collection (collection),
    FOREIGN KEY (variable) REFERENCES variables (id) ON DELETE CASCADE,
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE,
    FOREIGN KEY (environment) REFERENCES environments (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
