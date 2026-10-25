-- What folders and projects set for their APIs (#66): auth, and a folder's
-- headers, as JSON beside their rows. Only those that set something have a
-- row, which goes with its folder or project.
CREATE TABLE IF NOT EXISTS folder_settings (
    folder VARCHAR(64) PRIMARY KEY REFERENCES folders (id) ON DELETE CASCADE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    settings TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS folder_settings_collection ON folder_settings (collection);

CREATE TABLE IF NOT EXISTS project_settings (
    collection VARCHAR(64) PRIMARY KEY REFERENCES collections (id) ON DELETE CASCADE,
    settings TEXT NOT NULL
);
