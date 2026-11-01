-- G5: the PostgreSQL step, column for column. VARCHAR(255) keeps a unique
-- index of two utf8mb4 columns under InnoDB's 3072-byte limit
-- (255 * 4 * 2 = 2040). Issuer and subject must each fit 255 characters.
-- Many unbound rows (both NULL) are allowed.
--
-- MySQL has no ADD COLUMN IF NOT EXISTS here, and a stored procedure's
-- semicolons would be split apart, so each change is chosen from
-- information_schema and prepared. A step cut short is run again: every
-- statement below succeeds the second time too. ALTER TABLE commits on
-- its own; that is why a failed step is deleted and tried again.

SET @oidc_issuer = (
    SELECT IF(
        COUNT(*) = 0,
        'ALTER TABLE accounts ADD COLUMN oidc_issuer VARCHAR(255) NULL',
        'SELECT 1'
    )
    FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'accounts'
      AND COLUMN_NAME = 'oidc_issuer'
);
PREPARE oidc_issuer_stmt FROM @oidc_issuer;
EXECUTE oidc_issuer_stmt;
DEALLOCATE PREPARE oidc_issuer_stmt;

SET @oidc_subject = (
    SELECT IF(
        COUNT(*) = 0,
        'ALTER TABLE accounts ADD COLUMN oidc_subject VARCHAR(255) NULL',
        'SELECT 1'
    )
    FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'accounts'
      AND COLUMN_NAME = 'oidc_subject'
);
PREPARE oidc_subject_stmt FROM @oidc_subject;
EXECUTE oidc_subject_stmt;
DEALLOCATE PREPARE oidc_subject_stmt;

SET @oidc_index = (
    SELECT IF(
        COUNT(*) = 0,
        'ALTER TABLE accounts ADD UNIQUE INDEX accounts_oidc_identity (oidc_issuer, oidc_subject)',
        'SELECT 1'
    )
    FROM information_schema.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'accounts'
      AND INDEX_NAME = 'accounts_oidc_identity'
);
PREPARE oidc_index_stmt FROM @oidc_index;
EXECUTE oidc_index_stmt;
DEALLOCATE PREPARE oidc_index_stmt;
