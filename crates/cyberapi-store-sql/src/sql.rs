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
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, 'team', tm.role \
    FROM collections c JOIN team_members tm ON tm.team = c.owner_team WHERE tm.account = $1 \
    UNION ALL \
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, 'grant', g.role \
    FROM collections c JOIN collection_grants g ON g.collection = c.id \
    WHERE g.grantee_kind = 'account' AND g.grantee = $2 \
    UNION ALL \
    SELECT c.seq, c.id, c.name, c.description, c.created_at, c.updated_at, c.owner_team, 'grant', g.role \
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
    "SELECT id, name, description, created_at, updated_at, owner_team FROM collections WHERE id = $1";
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

// ---- variables -----------------------------------------------------------------

pub const VARIABLES_OF: &str = "SELECT id, collection, category, name, value, enabled, created_at, updated_at \
    FROM variables WHERE collection = $1 AND category = $2 ORDER BY seq";
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

// ---- the conformance probe -------------------------------------------------------

#[cfg(any(test, feature = "conformance"))]
pub const PROBE_COUNTS: [&str; 7] = [
    "SELECT COUNT(*) FROM collections WHERE id = $1",
    "SELECT COUNT(*) FROM folders WHERE collection = $1",
    "SELECT COUNT(*) FROM apis WHERE collection = $1",
    "SELECT COUNT(*) FROM variables WHERE collection = $1",
    "SELECT COUNT(*) FROM top_orders WHERE collection = $1",
    "SELECT COUNT(*) FROM collection_grants WHERE collection = $1",
    "SELECT COUNT(*) FROM prefs WHERE scope = $1",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every single-spelling statement, for the placeholder check.
    const ALL: &[&str] = &[
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
        TEAM_INSERT,
        PERSONAL_TEAM,
        TEAM_ROLE,
        MEMBER_DELETE,
        GRANT_DELETE,
        FOLDERS_OF,
        APIS_OF,
        API_GET,
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
    ];

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
        for pair in [MEMBER_UPSERT, GRANT_UPSERT, TOP_ORDER_UPSERT, PREF_UPSERT] {
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
