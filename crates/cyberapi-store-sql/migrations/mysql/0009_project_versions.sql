-- G4: a project as it was before each time it was replaced whole (a sync,
-- an upload, a version put back): its configuration, as JSON, newest by
-- `seq`; a project keeps the newest 50. `by` is the account's name then.
CREATE TABLE IF NOT EXISTS project_versions (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL,
    revision BIGINT NOT NULL,
    saved_at VARCHAR(40) NOT NULL,
    saved_by VARCHAR(255) NOT NULL,
    folders INT NOT NULL,
    apis INT NOT NULL,
    doc LONGTEXT NOT NULL,
    KEY project_versions_collection (collection, seq),
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
