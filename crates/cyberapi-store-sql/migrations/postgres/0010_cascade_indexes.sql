-- PERF-14: what deleting an API or an environment takes with it is found
-- by an index. PostgreSQL looks for the rows a cascade removes by the
-- column that refers, and without an index that starts with it reads the
-- whole table once for each row deleted: deleting a project of 2000 APIs
-- read `responses` 2000 times.
CREATE INDEX IF NOT EXISTS responses_api_only ON responses (api);
CREATE INDEX IF NOT EXISTS variable_environments_environment ON variable_environments (environment);
