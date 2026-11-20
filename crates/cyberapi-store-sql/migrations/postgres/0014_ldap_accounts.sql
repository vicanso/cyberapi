-- LDAP accounts: made by an LDAP sign-in, their password is the
-- directory's. Nobody sets one here, an administrator included.
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS ldap INTEGER NOT NULL DEFAULT 0;
