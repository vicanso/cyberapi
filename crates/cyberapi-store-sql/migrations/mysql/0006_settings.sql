-- What folders and projects set for their APIs (#66): auth, and a folder's
-- headers, as JSON beside their rows. Only those that set something have a
-- row, which goes with its folder or project.
CREATE TABLE IF NOT EXISTS folder_settings (
    folder VARCHAR(64) NOT NULL PRIMARY KEY,
    collection VARCHAR(64) NOT NULL,
    settings MEDIUMTEXT NOT NULL,
    KEY folder_settings_collection (collection),
    FOREIGN KEY (folder) REFERENCES folders (id) ON DELETE CASCADE,
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE IF NOT EXISTS project_settings (
    collection VARCHAR(64) NOT NULL PRIMARY KEY,
    settings MEDIUMTEXT NOT NULL,
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
