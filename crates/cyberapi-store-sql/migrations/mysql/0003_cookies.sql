-- Each account's cookie jar (M9): a cookie's document by the sha256 of its
-- domain, path and name (which together may be longer than a key can be).
CREATE TABLE IF NOT EXISTS cookies (
    account VARCHAR(64) NOT NULL,
    ckey VARCHAR(64) NOT NULL,
    doc MEDIUMTEXT NOT NULL,
    PRIMARY KEY (account, ckey),
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
