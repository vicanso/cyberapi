//! Every statement the store sends, and how each database takes it.
//!
//! sqlx's `Any` passes a statement through as written, and the two
//! databases number their placeholders differently: PostgreSQL `$1, $2`,
//! MySQL `?`. Each statement here is written once, in PostgreSQL's form,
//! numbering its placeholders from 1 in the order they appear — a test holds
//! every one to that — so MySQL's form is the same text with each `$n` made
//! `?`, bound in the same order. Only what the two spell differently — an
//! upsert — is written twice, as a [`Pair`].

use sqlx::AssertSqlSafe;
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    MySql,
}

/// A statement the two databases spell differently.
pub struct Pair {
    pub pg: &'static str,
    pub my: &'static str,
}

impl Dialect {
    /// The dialect of a connection, by the name sqlx gives its backend.
    pub fn of(backend: &str) -> Option<Self> {
        match backend {
            "PostgreSQL" => Some(Self::Postgres),
            "MySQL" => Some(Self::MySql),
            _ => None,
        }
    }

    /// `statement` as this database takes it. Every statement is one of the
    /// constants below, never built from input, which is what makes the
    /// assertion true.
    pub fn sql(self, statement: &'static str) -> AssertSqlSafe<Cow<'static, str>> {
        AssertSqlSafe(match self {
            Self::Postgres => Cow::Borrowed(statement),
            Self::MySql => Cow::Owned(question_marks(statement)),
        })
    }

    pub fn pick(self, pair: &Pair) -> AssertSqlSafe<Cow<'static, str>> {
        AssertSqlSafe(Cow::Borrowed(match self {
            Self::Postgres => pair.pg,
            Self::MySql => pair.my,
        }))
    }
}

fn question_marks(statement: &str) -> String {
    let mut out = String::with_capacity(statement.len());
    let mut chars = statement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            out.push('?');
        } else {
            out.push(ch);
        }
    }
    out
}

// ---- projects and who sees them --------------------------------------------

/// Every project an account sees, once per way it is reached: as a member
/// of the owning team (`team`, with the team role) or by a grant to the
/// account or one of its teams (`grant`, with the project role). The account
/// is bound three times.
pub const COLLECTIONS_VISIBLE: &str = "\
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, c.revision, 'team', tm.role \
    FROM collections c JOIN team_members tm ON tm.team = c.owner_team WHERE tm.account = $1 \
    UNION ALL \
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, c.revision, 'grant', g.role \
    FROM collections c JOIN collection_grants g ON g.collection = c.id \
    WHERE g.grantee_kind = 'account' AND g.grantee = $2 \
    UNION ALL \
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, c.revision, 'grant', g.role \
    FROM collections c JOIN collection_grants g ON g.collection = c.id \
    JOIN team_members tm ON tm.team = g.grantee \
    WHERE g.grantee_kind = 'team' AND tm.account = $3 \
    ORDER BY 1";

/// The ways one account reaches one project, as [`COLLECTIONS_VISIBLE`]
/// answers them. Bound: collection, account, three times.
pub const ROLE_OF: &str = "\
    SELECT 'team', tm.role FROM collections c JOIN team_members tm ON tm.team = c.owner_team \
    WHERE c.id = $1 AND tm.account = $2 \
    UNION ALL \
    SELECT 'grant', g.role FROM collection_grants g \
    WHERE g.collection = $3 AND g.grantee_kind = 'account' AND g.grantee = $4 \
    UNION ALL \
    SELECT 'grant', g.role FROM collection_grants g JOIN team_members tm ON tm.team = g.grantee \
    WHERE g.collection = $5 AND g.grantee_kind = 'team' AND tm.account = $6";

pub const COLLECTION_GET: &str =
    "SELECT id, name, description, created_at, updated_at, owner_team, revision FROM collections WHERE id = $1";
/// Holds the project's row for the transaction: every change to a project's
/// tree takes it first, so two cannot plan from the same lists (§7).
pub const COLLECTION_LOCK: &str = "SELECT id FROM collections WHERE id = $1 FOR UPDATE";
pub const COLLECTION_INSERT: &str = "INSERT INTO collections \
    (id, owner_team, name, description, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)";
pub const COLLECTION_UPDATE: &str = "UPDATE collections \
    SET name = $1, description = $2, updated_at = $3, revision = revision + 1 WHERE id = $4";
/// A change to the project's content (#21, §7).
pub const COLLECTION_TOUCH: &str = "UPDATE collections SET updated_at = $1, revision = revision + 1 WHERE id = $2";
pub const COLLECTION_DELETE: &str = "DELETE FROM collections WHERE id = $1";

// ---- accounts and teams ------------------------------------------------------

pub const ACCOUNT_INSERT: &str = "INSERT INTO accounts \
    (id, name, display_name, password_hash, admin, created_at) VALUES ($1, $2, $3, $4, $5, $6)";
pub const ACCOUNT_COUNT: &str = "SELECT COUNT(*) FROM accounts";
pub const ACCOUNT_LOGIN: &str = "SELECT id, name, display_name, admin, password_hash \
    FROM accounts WHERE name = $1 AND disabled = 0";
pub const SESSION_INSERT: &str =
    "INSERT INTO sessions (token_hash, account, expires_at, created_at) VALUES ($1, $2, $3, $4)";
pub const SESSION_ACCOUNT: &str = "SELECT a.id, a.name, a.display_name, a.admin \
    FROM sessions s JOIN accounts a ON a.id = s.account \
    WHERE s.token_hash = $1 AND s.expires_at > $2 AND a.disabled = 0";
pub const SESSION_DELETE: &str = "DELETE FROM sessions WHERE token_hash = $1";
pub const SESSIONS_EXPIRED_DELETE: &str = "DELETE FROM sessions WHERE expires_at <= $1";
/// Claims the making of the first administrator. Of two setups at once,
/// one inserts the row; the other waits for it and inserts nothing.
pub const SETUP_CLAIM: Pair = Pair {
    pg: "INSERT INTO store_meta (name, value) VALUES ('setup', $1) ON CONFLICT (name) DO NOTHING",
    my: "INSERT IGNORE INTO store_meta (name, value) VALUES ('setup', ?)",
};
pub const TEAM_INSERT: &str = "INSERT INTO teams (id, name, personal_of, created_at) VALUES ($1, $2, $3, $4)";
pub const PERSONAL_TEAM: &str = "SELECT id FROM teams WHERE personal_of = $1";
pub const TEAM_ROLE: &str = "SELECT role FROM team_members WHERE team = $1 AND account = $2";
pub const MEMBER_UPSERT: Pair = Pair {
    pg: "INSERT INTO team_members (team, account, role) VALUES ($1, $2, $3) \
         ON CONFLICT (team, account) DO UPDATE SET role = EXCLUDED.role",
    my: "INSERT INTO team_members (team, account, role) VALUES (?, ?, ?) AS new \
         ON DUPLICATE KEY UPDATE role = new.role",
};
pub const MEMBER_DELETE: &str = "DELETE FROM team_members WHERE team = $1 AND account = $2";
pub const GRANT_UPSERT: Pair = Pair {
    pg: "INSERT INTO collection_grants (collection, grantee_kind, grantee, role) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (collection, grantee_kind, grantee) DO UPDATE SET role = EXCLUDED.role",
    my: "INSERT INTO collection_grants (collection, grantee_kind, grantee, role) VALUES (?, ?, ?, ?) AS new \
         ON DUPLICATE KEY UPDATE role = new.role",
};
pub const GRANT_DELETE: &str =
    "DELETE FROM collection_grants WHERE collection = $1 AND grantee_kind = $2 AND grantee = $3";

// ---- the directory (§9.4) -----------------------------------------------------

pub const ACCOUNT_INFO: &str = "SELECT id, name, display_name, admin, disabled FROM accounts WHERE id = $1";
pub const ACCOUNTS_ALL: &str = "SELECT id, name, display_name, admin, disabled FROM accounts";
pub const ACCOUNT_NAME_COUNT: &str = "SELECT COUNT(*) FROM accounts WHERE name = $1";
pub const ACCOUNT_UPDATE: &str = "UPDATE accounts SET display_name = $1, admin = $2, disabled = $3 WHERE id = $4";
pub const ACCOUNT_PASSWORD: &str = "SELECT password_hash FROM accounts WHERE id = $1";
pub const ACCOUNT_SET_PASSWORD: &str = "UPDATE accounts SET password_hash = $1 WHERE id = $2";
pub const SESSIONS_OF_DELETE: &str = "DELETE FROM sessions WHERE account = $1";
pub const INVITE_INSERT: &str = "INSERT INTO invites \
    (id, token_hash, inviter, team, role, created_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7)";
/// The invitations an account sent that can still be used (at unix `$2`),
/// with their team's name.
pub const INVITES_OPEN_OF: &str = "SELECT i.id, i.team, t.name, i.role, i.created_at, i.expires_at \
    FROM invites i LEFT JOIN teams t ON t.id = i.team \
    WHERE i.inviter = $1 AND i.used_by IS NULL AND i.revoked = 0 AND i.expires_at > $2";
pub const INVITE_REVOKE: &str = "UPDATE invites SET revoked = 1 \
    WHERE id = $1 AND inviter = $2 AND used_by IS NULL AND revoked = 0";
/// An invitation that can still be used: who sent it, and to which team.
pub const INVITE_OPEN: &str = "SELECT a.name, a.display_name, t.name \
    FROM invites i JOIN accounts a ON a.id = i.inviter LEFT JOIN teams t ON t.id = i.team \
    WHERE i.token_hash = $1 AND i.used_by IS NULL AND i.revoked = 0 AND i.expires_at > $2";
/// Uses an invitation for account `$1`: of two acceptances at once, one
/// claims it.
pub const INVITE_CLAIM: &str = "UPDATE invites SET used_by = $1 \
    WHERE token_hash = $2 AND used_by IS NULL AND revoked = 0 AND expires_at > $3";
pub const INVITE_PLACE: &str = "SELECT inviter, team, role FROM invites WHERE token_hash = $1";
pub const MCP_TOKEN_INSERT: &str = "INSERT INTO mcp_tokens \
    (id, token_hash, account, name, can_send, created_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7)";
/// An account's tokens that still sign in at unix `$2`.
pub const MCP_TOKENS_OF: &str = "SELECT id, name, can_send, created_at, expires_at FROM mcp_tokens \
    WHERE account = $1 AND (expires_at IS NULL OR expires_at > $2)";
pub const MCP_TOKEN_NAME_COUNT: &str = "SELECT COUNT(*) FROM mcp_tokens WHERE account = $1 AND name = $2";
pub const MCP_TOKEN_DELETE: &str = "DELETE FROM mcp_tokens WHERE id = $1 AND account = $2";
/// The enabled account a token signs in as at unix `$2`, and the token.
pub const MCP_TOKEN_ACCOUNT: &str = "SELECT a.id, a.name, a.display_name, a.admin, \
    t.id, t.name, t.can_send, t.created_at, t.expires_at \
    FROM mcp_tokens t JOIN accounts a ON a.id = t.account \
    WHERE t.token_hash = $1 AND (t.expires_at IS NULL OR t.expires_at > $2) AND a.disabled = 0";
/// The teams an account is in, with its place in each.
pub const TEAMS_OF: &str = "SELECT t.id, t.name, t.personal_of, tm.role \
    FROM teams t JOIN team_members tm ON tm.team = t.id WHERE tm.account = $1";
/// A team and the account's place in it, if it is in it.
pub const TEAM_OF: &str = "SELECT t.id, t.name, t.personal_of, tm.role \
    FROM teams t JOIN team_members tm ON tm.team = t.id WHERE t.id = $1 AND tm.account = $2";
pub const TEAM_EXISTS: &str = "SELECT COUNT(*) FROM teams WHERE id = $1";
/// Holds a team's members still while one change to them is checked and
/// made: two owners cannot each step down, leaving none.
pub const TEAM_LOCK: &str = "SELECT id FROM teams WHERE id = $1 FOR UPDATE";
pub const TEAM_RENAME: &str = "UPDATE teams SET name = $1 WHERE id = $2";
pub const TEAM_DELETE: &str = "DELETE FROM teams WHERE id = $1";
pub const TEAM_GRANTS_DELETE: &str = "DELETE FROM collection_grants WHERE grantee_kind = 'team' AND grantee = $1";
pub const TEAM_PROJECT_COUNT: &str = "SELECT COUNT(*) FROM collections WHERE owner_team = $1";
pub const TEAM_OWNER_COUNT: &str = "SELECT COUNT(*) FROM team_members WHERE team = $1 AND role = 'owner'";
pub const TEAM_MEMBERS: &str = "SELECT a.id, a.name, a.display_name, a.admin, a.disabled, tm.role \
    FROM team_members tm JOIN accounts a ON a.id = tm.account WHERE tm.team = $1";
/// A project's grants, with what each grantee is called.
pub const GRANTS_OF: &str = "SELECT g.grantee_kind, g.grantee, g.role, a.name, a.display_name, t.name \
    FROM collection_grants g \
    LEFT JOIN accounts a ON g.grantee_kind = 'account' AND a.id = g.grantee \
    LEFT JOIN teams t ON g.grantee_kind = 'team' AND t.id = g.grantee \
    WHERE g.collection = $1";
pub const COLLECTION_TRANSFER: &str = "UPDATE collections SET owner_team = $1 WHERE id = $2";

// ---- the tree ----------------------------------------------------------------

pub const FOLDERS_OF: &str = "SELECT id, collection, children, name, created_at, updated_at \
    FROM folders WHERE collection = $1 ORDER BY seq";
pub const APIS_OF: &str = "SELECT id, collection, name, category, request, created_at, updated_at \
    FROM apis WHERE collection = $1 ORDER BY seq";
pub const API_GET: &str = "SELECT id, collection, name, category, request, created_at, updated_at \
    FROM apis WHERE id = $1 AND collection = $2";
pub const TOP_ORDER_OF: &str = "SELECT items FROM top_orders WHERE collection = $1";
pub const TOP_ORDER_UPSERT: Pair = Pair {
    pg: "INSERT INTO top_orders (collection, items) VALUES ($1, $2) \
         ON CONFLICT (collection) DO UPDATE SET items = EXCLUDED.items",
    my: "INSERT INTO top_orders (collection, items) VALUES (?, ?) AS new \
         ON DUPLICATE KEY UPDATE items = new.items",
};
pub const FOLDER_INSERT: &str = "INSERT INTO folders \
    (id, collection, name, children, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)";
pub const API_INSERT: &str = "INSERT INTO apis \
    (id, collection, name, category, request, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)";
pub const FOLDER_SET_CHILDREN: &str = "UPDATE folders SET children = $1 WHERE id = $2 AND collection = $3";
pub const FOLDER_RENAME: &str = "UPDATE folders SET name = $1, updated_at = $2 WHERE id = $3 AND collection = $4";
pub const API_RENAME: &str = "UPDATE apis SET name = $1, updated_at = $2 WHERE id = $3 AND collection = $4";
pub const API_SAVE: &str = "UPDATE apis SET request = $1, updated_at = $2 WHERE id = $3 AND collection = $4";
pub const FOLDER_DELETE: &str = "DELETE FROM folders WHERE id = $1 AND collection = $2";
pub const API_DELETE: &str = "DELETE FROM apis WHERE id = $1 AND collection = $2";
pub const API_COUNTS: &str = "SELECT collection, COUNT(*) FROM apis GROUP BY collection";

// A whole configuration put in place of a project's (server-sync §3.3).
// Folders and variables are written again; an API that stays is rewritten
// in its row, which its responses hang from.
pub const FOLDER_OWNER: &str = "SELECT collection FROM folders WHERE id = $1";
pub const API_OWNER: &str = "SELECT collection FROM apis WHERE id = $1";
pub const VARIABLE_OWNER: &str = "SELECT collection FROM variables WHERE id = $1";
pub const API_IDS_OF: &str = "SELECT id FROM apis WHERE collection = $1";
pub const API_REWRITE: &str = "UPDATE apis \
    SET name = $1, category = $2, request = $3, created_at = $4, updated_at = $5 WHERE id = $6 AND collection = $7";
pub const FOLDERS_DELETE_OF: &str = "DELETE FROM folders WHERE collection = $1";
pub const VARIABLES_DELETE_OF: &str = "DELETE FROM variables WHERE collection = $1";

// ---- variables -----------------------------------------------------------------

pub const VARIABLES_OF: &str = "SELECT id, collection, category, name, value, enabled, created_at, updated_at \
    FROM variables WHERE collection = $1 AND category = $2 ORDER BY seq";
pub const VARIABLES_ALL_OF: &str = "SELECT id, collection, category, name, value, enabled, created_at, updated_at \
    FROM variables WHERE collection = $1 ORDER BY seq";
pub const VARIABLE_GET: &str = "SELECT id, collection, category, name, value, enabled, created_at, updated_at \
    FROM variables WHERE id = $1 AND collection = $2";
pub const VARIABLE_INSERT: &str = "INSERT INTO variables \
    (id, collection, category, name, value, enabled, created_at, updated_at) \
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
pub const VARIABLE_UPDATE: &str = "UPDATE variables \
    SET name = $1, value = $2, enabled = $3, updated_at = $4 WHERE id = $5 AND collection = $6";
pub const VARIABLE_DELETE: &str = "DELETE FROM variables WHERE id = $1 AND collection = $2";

// ---- what each account keeps ---------------------------------------------------

pub const PREF_GET: &str = "SELECT doc FROM prefs WHERE account = $1 AND scope = $2";
pub const PREF_UPSERT: Pair = Pair {
    pg: "INSERT INTO prefs (account, scope, doc) VALUES ($1, $2, $3) \
         ON CONFLICT (account, scope) DO UPDATE SET doc = EXCLUDED.doc",
    my: "INSERT INTO prefs (account, scope, doc) VALUES (?, ?, ?) AS new \
         ON DUPLICATE KEY UPDATE doc = new.doc",
};
/// Every account's state of one project, when the project goes.
pub const PREFS_DELETE_SCOPE: &str = "DELETE FROM prefs WHERE scope = $1";
/// The account's state of every project; its `app` settings stay.
pub const PREFS_CLEAR_WORKSPACES: &str = "DELETE FROM prefs WHERE account = $1 AND scope LIKE 'workspace/%'";

// The responses each API keeps, per account, newest by `seq`. They go with
// their API, project and account by the foreign keys.
pub const RESPONSE_INSERT: &str = "INSERT INTO responses \
    (id, account, collection, api, doc, created_at) VALUES ($1, $2, $3, $4, $5, $6)";
pub const RESPONSES_OF: &str =
    "SELECT doc FROM responses WHERE account = $1 AND collection = $2 AND api = $3 ORDER BY seq DESC";
pub const RESPONSE_NEWEST: &str =
    "SELECT doc FROM responses WHERE account = $1 AND collection = $2 AND api = $3 ORDER BY seq DESC LIMIT 1";
/// The oldest response within the limit (`HISTORY_LIMIT - 1` newer ones).
pub const RESPONSE_OLDEST_KEPT: &str =
    "SELECT seq FROM responses WHERE account = $1 AND collection = $2 AND api = $3 ORDER BY seq DESC LIMIT 1 OFFSET 9";
pub const RESPONSES_TRIM: &str =
    "DELETE FROM responses WHERE account = $1 AND collection = $2 AND api = $3 AND seq < $4";
pub const RESPONSES_CLEAR: &str = "DELETE FROM responses WHERE account = $1 AND collection = $2 AND api = $3";
pub const RESPONSES_CLEAR_ALL: &str = "DELETE FROM responses WHERE account = $1";

// Each account's cookie jar.
pub const COOKIES_OF: &str = "SELECT doc FROM cookies WHERE account = $1";
pub const COOKIE_UPSERT: Pair = Pair {
    pg: "INSERT INTO cookies (account, ckey, doc) VALUES ($1, $2, $3) \
         ON CONFLICT (account, ckey) DO UPDATE SET doc = EXCLUDED.doc",
    my: "INSERT INTO cookies (account, ckey, doc) VALUES (?, ?, ?) AS new \
         ON DUPLICATE KEY UPDATE doc = new.doc",
};
pub const COOKIE_DELETE: &str = "DELETE FROM cookies WHERE account = $1 AND ckey = $2";
pub const COOKIES_CLEAR: &str = "DELETE FROM cookies WHERE account = $1";

// ---- the conformance probe -------------------------------------------------------

#[cfg(any(test, feature = "conformance"))]
pub const PROBE_COUNTS: [&str; 8] = [
    "SELECT COUNT(*) FROM collections WHERE id = $1",
    "SELECT COUNT(*) FROM folders WHERE collection = $1",
    "SELECT COUNT(*) FROM apis WHERE collection = $1",
    "SELECT COUNT(*) FROM variables WHERE collection = $1",
    "SELECT COUNT(*) FROM top_orders WHERE collection = $1",
    "SELECT COUNT(*) FROM collection_grants WHERE collection = $1",
    "SELECT COUNT(*) FROM responses WHERE collection = $1",
    // Last: bound to the workspace scope, not the id.
    "SELECT COUNT(*) FROM prefs WHERE scope = $1",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every single-spelling statement, for the placeholder check.
    const ALL: &[&str] = &[
        INVITE_INSERT,
        INVITES_OPEN_OF,
        INVITE_REVOKE,
        INVITE_OPEN,
        INVITE_CLAIM,
        INVITE_PLACE,
        MCP_TOKEN_INSERT,
        MCP_TOKENS_OF,
        MCP_TOKEN_NAME_COUNT,
        MCP_TOKEN_DELETE,
        MCP_TOKEN_ACCOUNT,
        COLLECTIONS_VISIBLE,
        ROLE_OF,
        COLLECTION_GET,
        COLLECTION_LOCK,
        COLLECTION_INSERT,
        COLLECTION_UPDATE,
        COLLECTION_TOUCH,
        COLLECTION_DELETE,
        ACCOUNT_INSERT,
        ACCOUNT_COUNT,
        ACCOUNT_LOGIN,
        SESSION_INSERT,
        SESSION_ACCOUNT,
        SESSION_DELETE,
        SESSIONS_EXPIRED_DELETE,
        TEAM_INSERT,
        PERSONAL_TEAM,
        TEAM_ROLE,
        MEMBER_DELETE,
        GRANT_DELETE,
        ACCOUNT_INFO,
        ACCOUNTS_ALL,
        ACCOUNT_NAME_COUNT,
        ACCOUNT_UPDATE,
        ACCOUNT_PASSWORD,
        ACCOUNT_SET_PASSWORD,
        SESSIONS_OF_DELETE,
        TEAMS_OF,
        TEAM_OF,
        TEAM_EXISTS,
        TEAM_LOCK,
        TEAM_RENAME,
        TEAM_DELETE,
        TEAM_GRANTS_DELETE,
        TEAM_PROJECT_COUNT,
        TEAM_OWNER_COUNT,
        TEAM_MEMBERS,
        GRANTS_OF,
        COLLECTION_TRANSFER,
        FOLDERS_OF,
        APIS_OF,
        FOLDER_OWNER,
        API_OWNER,
        VARIABLE_OWNER,
        API_IDS_OF,
        API_REWRITE,
        FOLDERS_DELETE_OF,
        VARIABLES_DELETE_OF,
        VARIABLES_ALL_OF,
        API_GET,
        API_COUNTS,
        TOP_ORDER_OF,
        FOLDER_INSERT,
        API_INSERT,
        FOLDER_SET_CHILDREN,
        FOLDER_RENAME,
        API_RENAME,
        API_SAVE,
        FOLDER_DELETE,
        API_DELETE,
        VARIABLES_OF,
        VARIABLE_GET,
        VARIABLE_INSERT,
        VARIABLE_UPDATE,
        VARIABLE_DELETE,
        PREF_GET,
        PREFS_DELETE_SCOPE,
        PREFS_CLEAR_WORKSPACES,
        RESPONSE_INSERT,
        RESPONSES_OF,
        RESPONSE_NEWEST,
        RESPONSE_OLDEST_KEPT,
        RESPONSES_TRIM,
        RESPONSES_CLEAR,
        RESPONSES_CLEAR_ALL,
        COOKIES_OF,
        COOKIE_DELETE,
        COOKIES_CLEAR,
    ];

    #[test]
    fn the_history_trim_keeps_the_limit() {
        let offset = cyberapi_core::response::HISTORY_LIMIT - 1;
        assert!(RESPONSE_OLDEST_KEPT.ends_with(&format!("OFFSET {offset}")));
    }

    fn placeholders(statement: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut chars = statement.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' {
                let mut number = String::new();
                while let Some(digit) = chars.peek().filter(|c| c.is_ascii_digit()) {
                    number.push(*digit);
                    chars.next();
                }
                found.push(number.parse().unwrap_or(0));
            }
        }
        found
    }

    #[test]
    fn placeholders_count_up_from_one_in_order() {
        // MySQL binds `?` by position, so `$n` must be the n-th placeholder.
        for statement in ALL.iter().chain(PROBE_COUNTS.iter()) {
            let found = placeholders(statement);
            let expected: Vec<usize> = (1..=found.len()).collect();
            assert_eq!(found, expected, "{statement}");
        }
        for pair in [MEMBER_UPSERT, GRANT_UPSERT, TOP_ORDER_UPSERT, PREF_UPSERT, SETUP_CLAIM] {
            assert_eq!(placeholders(pair.pg).len(), pair.my.matches('?').count(), "{}", pair.pg);
        }
    }

    #[test]
    fn mysql_gets_question_marks() {
        assert_eq!(
            question_marks("a = $1 AND b = $12, c = '$'"),
            "a = ? AND b = ?, c = '$'"
        );
    }
}
