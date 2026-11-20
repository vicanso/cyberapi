-- H3: settings an administrator makes on the page for the whole bridge,
-- one JSON document a name (`ai`: the assistant's model, its key with it).
CREATE TABLE IF NOT EXISTS bridge_settings (
    name VARCHAR(64) PRIMARY KEY,
    value TEXT NOT NULL
);
