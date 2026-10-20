-- The responses each API keeps (F7.9): every account its own, the newest
-- by `seq`. They go with their API, project or account.
CREATE TABLE IF NOT EXISTS responses (
    seq BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    account VARCHAR(64) NOT NULL,
    collection VARCHAR(64) NOT NULL,
    api VARCHAR(64) NOT NULL,
    doc LONGTEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    KEY responses_api (account, api, seq),
    KEY responses_collection (collection),
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE,
    FOREIGN KEY (collection) REFERENCES collections (id) ON DELETE CASCADE,
    FOREIGN KEY (api) REFERENCES apis (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
