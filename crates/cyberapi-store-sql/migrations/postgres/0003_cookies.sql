-- Each account's cookie jar (M9): a cookie's document by the sha256 of its
-- domain, path and name (which together may be longer than a key can be).
CREATE TABLE IF NOT EXISTS cookies (
    account VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    ckey VARCHAR(64) NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (account, ckey)
);
