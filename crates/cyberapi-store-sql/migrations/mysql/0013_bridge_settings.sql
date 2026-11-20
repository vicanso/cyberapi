-- H3: settings an administrator makes on the page for the whole bridge,
-- one JSON document a name (`ai`: the assistant's model, its key with it).
CREATE TABLE IF NOT EXISTS bridge_settings (
    name VARCHAR(64) NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
