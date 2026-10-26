-- G4: a project as it was before each time it was replaced whole (a sync,
-- an upload, a version put back): its configuration, as JSON, newest by
-- `seq`; a project keeps the newest 50. `by` is the account's name then.
CREATE TABLE IF NOT EXISTS project_versions (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    revision BIGINT NOT NULL,
    saved_at VARCHAR(40) NOT NULL,
    saved_by TEXT NOT NULL,
    folders INTEGER NOT NULL,
    apis INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS project_versions_collection ON project_versions (collection, seq);
