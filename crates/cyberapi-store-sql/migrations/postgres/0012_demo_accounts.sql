-- Demo accounts: may sign in and use the app, but cannot change their own
-- password (an administrator can still reset it).
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS demo INTEGER NOT NULL DEFAULT 0;
