-- LDAP accounts: the PostgreSQL step, column for column.
--
-- MySQL has no ADD COLUMN IF NOT EXISTS here, so the change is chosen from
-- information_schema and prepared. A step cut short is run again safely.

SET @ldap_col = (
    SELECT IF(
        COUNT(*) = 0,
        'ALTER TABLE accounts ADD COLUMN ldap INT NOT NULL DEFAULT 0',
        'SELECT 1'
    )
    FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'accounts'
      AND COLUMN_NAME = 'ldap'
);
PREPARE ldap_col_stmt FROM @ldap_col;
EXECUTE ldap_col_stmt;
DEALLOCATE PREPARE ldap_col_stmt;
