-- G5: an OpenID Connect subject on an account. Both columns stay NULL
-- until the account is linked on its first sign-in. A unique index treats
-- each NULL as distinct, so many unbound accounts are allowed. Issuer and
-- subject must each fit 255 characters.
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS oidc_issuer VARCHAR(255);
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS oidc_subject VARCHAR(255);
CREATE UNIQUE INDEX IF NOT EXISTS accounts_oidc_identity ON accounts (oidc_issuer, oidc_subject);
