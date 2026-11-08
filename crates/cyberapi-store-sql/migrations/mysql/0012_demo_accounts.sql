-- Demo accounts: the PostgreSQL step, column for column.
--
-- MySQL has no ADD COLUMN IF NOT EXISTS here, so the change is chosen from
-- information_schema and prepared. A step cut short is run again safely.

SET @demo_col = (
    SELECT IF(
        COUNT(*) = 0,
        'ALTER TABLE accounts ADD COLUMN demo INT NOT NULL DEFAULT 0',
        'SELECT 1'
    )
    FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'accounts'
      AND COLUMN_NAME = 'demo'
);
PREPARE demo_col_stmt FROM @demo_col;
EXECUTE demo_col_stmt;
DEALLOCATE PREPARE demo_col_stmt;
