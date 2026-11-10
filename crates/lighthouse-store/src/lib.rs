//! SQLite storage for Lighthouse: dashboard users and sessions, and server status history.
//!
//! One file (`database` in the config). All calls are short, so the connection sits behind a plain
//! mutex; callers in async code can call these directly.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use lighthouse_core::{ServerState, ServerStatus};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Schema migrations, applied in order. Never edit an existing entry; append a new one.
const MIGRATIONS: &[&str] = &[
    // 1: users, sessions, status history
    "CREATE TABLE users (
        id            INTEGER PRIMARY KEY,
        username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
        password_hash TEXT NOT NULL,
        is_superadmin INTEGER NOT NULL DEFAULT 0,
        permissions   TEXT NOT NULL DEFAULT '[]',
        servers       TEXT,
        created_at    INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        token      TEXT PRIMARY KEY,
        user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        expires_at INTEGER NOT NULL
    );
    CREATE TABLE status_history (
        server_id   TEXT NOT NULL,
        at          INTEGER NOT NULL,
        state       TEXT NOT NULL,
        players     INTEGER,
        max_players INTEGER
    );
    CREATE INDEX status_history_server_at ON status_history(server_id, at);",
    // 2: invite links (the only way to register)
    "CREATE TABLE invites (
        code_hash   TEXT PRIMARY KEY,
        permissions TEXT NOT NULL DEFAULT '[]',
        servers     TEXT,
        created_by  INTEGER NOT NULL,
        created_at  INTEGER NOT NULL,
        expires_at  INTEGER NOT NULL,
        used_by     INTEGER,
        used_at     INTEGER
    );",
    // 3: addresses blocked for guessing invite codes
    "CREATE TABLE ip_blocks (
        ip            TEXT PRIMARY KEY,
        failures      INTEGER NOT NULL DEFAULT 0,
        threshold     INTEGER NOT NULL,
        strikes       INTEGER NOT NULL DEFAULT 0,
        blocked_until INTEGER,
        permanent     INTEGER NOT NULL DEFAULT 0,
        last_failure  INTEGER NOT NULL
    );",
];

/// How long each successive block lasts. After the last one, blocks are permanent until an admin
/// removes them.
pub const BLOCK_STEPS: [i64; 4] = [30 * 60, 3 * 3600, 7 * 86_400, 30 * 86_400];
/// Failed guesses older than this no longer count towards the next block.
const FAILURE_MEMORY: i64 = 24 * 3600;

/// A blocked (or watched) address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpBlock {
    pub ip: String,
    /// Failed guesses since the last block.
    pub failures: u32,
    /// How many times this address has been blocked.
    pub strikes: u32,
    /// `None` with `permanent = false` means not currently blocked.
    pub blocked_until: Option<i64>,
    pub permanent: bool,
    pub last_failure: i64,
}

impl IpBlock {
    pub fn is_blocked(&self, now: i64) -> bool {
        self.permanent || self.blocked_until.is_some_and(|t| t > now)
    }
}

/// A random number of allowed guesses (2-5) before the next block, so the limit can't be probed.
fn random_threshold() -> u32 {
    let mut b = [0u8; 1];
    let _ = getrandom::getrandom(&mut b);
    2 + u32::from(b[0] % 4)
}

fn block_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IpBlock> {
    Ok(IpBlock {
        ip: row.get(0)?,
        failures: row.get(1)?,
        strikes: row.get(2)?,
        blocked_until: row.get(3)?,
        permanent: row.get::<_, i64>(4)? != 0,
        last_failure: row.get(5)?,
    })
}

/// A single-use invite to create a dashboard account with preset access.
///
/// The code itself (256 random bits) is only shown once, when the invite is created. The database
/// keeps its SHA-256 hash, so a copy of the database can't be used to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// SHA-256 of the code, hex. Safe to show (e.g. in a "cancel invite" link).
    pub id: String,
    pub permissions: BTreeSet<String>,
    pub servers: Option<BTreeSet<String>>,
    pub created_by: i64,
    pub created_at: i64,
    pub expires_at: i64,
}

const INVITE_COLUMNS: &str = "code_hash, permissions, servers, created_by, created_at, expires_at";

fn invite_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Invite> {
    let permissions: String = row.get(1)?;
    let servers: Option<String> = row.get(2)?;
    Ok(Invite {
        id: row.get(0)?,
        permissions: serde_json::from_str(&permissions).unwrap_or_default(),
        servers: servers.and_then(|s| serde_json::from_str(&s).ok()),
        created_by: row.get(3)?,
        created_at: row.get(4)?,
        expires_at: row.get(5)?,
    })
}

/// A dashboard user. The superadmin can do everything; everyone else only what they were granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub is_superadmin: bool,
    /// Permission keys declared by dashboard modules, e.g. `logs.view`.
    pub permissions: BTreeSet<String>,
    /// Server ids this user may see. `None` = all servers.
    pub servers: Option<BTreeSet<String>>,
    pub created_at: i64,
}

impl User {
    pub fn can(&self, permission: &str) -> bool {
        self.is_superadmin || self.permissions.contains(permission)
    }

    pub fn sees_server(&self, server_id: &str) -> bool {
        self.is_superadmin || self.servers.as_ref().is_none_or(|s| s.contains(server_id))
    }
}

/// One stored status sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub at: i64,
    pub state: ServerState,
    pub players: Option<u32>,
    pub max_players: Option<u32>,
}

fn state_str(s: ServerState) -> &'static str {
    match s {
        ServerState::Online => "online",
        ServerState::Starting => "starting",
        ServerState::Offline => "offline",
        ServerState::Unknown => "unknown",
    }
}

fn state_from(s: &str) -> ServerState {
    match s {
        "online" => ServerState::Online,
        "starting" => ServerState::Starting,
        "offline" => ServerState::Offline,
        _ => ServerState::Unknown,
    }
}

fn hash_password(password: &str) -> Result<String> {
    if password.chars().count() < 8 {
        return Err(StoreError::Invalid("passwords need at least 8 characters".into()));
    }
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| StoreError::Hash(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| StoreError::Hash(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| StoreError::Hash(e.to_string()))
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_token() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| StoreError::Hash(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn valid_username(name: &str) -> bool {
    (2..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

const USER_COLUMNS: &str = "id, username, is_superadmin, permissions, servers, created_at";

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let permissions: String = row.get(3)?;
    let servers: Option<String> = row.get(4)?;
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        is_superadmin: row.get::<_, i64>(2)? != 0,
        permissions: serde_json::from_str(&permissions).unwrap_or_default(),
        servers: servers.and_then(|s| serde_json::from_str(&s).ok()),
        created_at: row.get(5)?,
    })
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            // New database: let pruning hand space back instead of keeping the file at its peak size.
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        }
        // WAL with normal syncing is safe and needs far fewer disk flushes; cap the WAL file at 4 MB.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA journal_size_limit = 4194304;
             PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA cache_size = -2000;",
        )?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(&format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1))?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---------- users ----------

    pub fn has_superadmin(&self) -> Result<bool> {
        Ok(self
            .db()
            .query_row("SELECT EXISTS(SELECT 1 FROM users WHERE is_superadmin = 1)", [], |r| {
                r.get(0)
            })?)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "SELECT {USER_COLUMNS} FROM users ORDER BY is_superadmin DESC, username"
        ))?;
        let rows = stmt.query_map([], user_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn user(&self, id: i64) -> Result<Option<User>> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1"),
                [id],
                user_from_row,
            )
            .optional()?)
    }

    /// Create a user. Sub-admins start with the given permissions and server scope.
    pub fn create_user(
        &self,
        username: &str,
        password: &str,
        is_superadmin: bool,
        permissions: &BTreeSet<String>,
        servers: Option<&BTreeSet<String>>,
        now: i64,
    ) -> Result<User> {
        let username = username.trim();
        if !valid_username(username) {
            return Err(StoreError::Invalid(
                "usernames are 2-32 letters, digits, '-', '_' or '.'".into(),
            ));
        }
        let hash = hash_password(password)?;
        let db = self.db();
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE username = ?1)",
            [username],
            |r| r.get(0),
        )?;
        if exists {
            return Err(StoreError::Invalid(format!("the username {username:?} is taken")));
        }
        db.execute(
            "INSERT INTO users (username, password_hash, is_superadmin, permissions, servers, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                username,
                hash,
                is_superadmin as i64,
                serde_json::to_string(permissions).expect("strings serialise"),
                servers.map(|s| serde_json::to_string(s).expect("strings serialise")),
                now
            ],
        )?;
        let id = db.last_insert_rowid();
        drop(db);
        self.user(id)?
            .ok_or_else(|| StoreError::Invalid("user vanished".into()))
    }

    /// Create the superadmin, or reset its password if one exists with this name.
    pub fn set_superadmin(&self, username: &str, password: &str, now: i64) -> Result<User> {
        let existing = self.users()?.into_iter().find(|u| u.is_superadmin);
        match existing {
            Some(u) => {
                self.set_password(u.id, password)?;
                if !u.username.eq_ignore_ascii_case(username.trim()) {
                    if !valid_username(username.trim()) {
                        return Err(StoreError::Invalid(
                            "usernames are 2-32 letters, digits, '-', '_' or '.'".into(),
                        ));
                    }
                    self.db().execute(
                        "UPDATE users SET username = ?1 WHERE id = ?2",
                        params![username.trim(), u.id],
                    )?;
                }
                self.user(u.id)?
                    .ok_or_else(|| StoreError::Invalid("user vanished".into()))
            }
            None => self.create_user(username, password, true, &BTreeSet::new(), None, now),
        }
    }

    pub fn set_access(
        &self,
        id: i64,
        permissions: &BTreeSet<String>,
        servers: Option<&BTreeSet<String>>,
    ) -> Result<()> {
        self.db().execute(
            "UPDATE users SET permissions = ?1, servers = ?2 WHERE id = ?3 AND is_superadmin = 0",
            params![
                serde_json::to_string(permissions).expect("strings serialise"),
                servers.map(|s| serde_json::to_string(s).expect("strings serialise")),
                id
            ],
        )?;
        Ok(())
    }

    pub fn set_password(&self, id: i64, password: &str) -> Result<()> {
        let hash = hash_password(password)?;
        let db = self.db();
        db.execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![hash, id])?;
        // Changing a password signs the user out everywhere.
        db.execute("DELETE FROM sessions WHERE user_id = ?1", [id])?;
        Ok(())
    }

    /// Delete a sub-admin. The superadmin cannot be deleted.
    pub fn delete_user(&self, id: i64) -> Result<()> {
        self.db()
            .execute("DELETE FROM users WHERE id = ?1 AND is_superadmin = 0", [id])?;
        Ok(())
    }

    /// Check a login. Returns the user on success.
    pub fn verify_login(&self, username: &str, password: &str) -> Result<Option<User>> {
        let row: Option<(i64, String)> = self
            .db()
            .query_row(
                "SELECT id, password_hash FROM users WHERE username = ?1",
                [username.trim()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((id, hash)) if verify_password(password, &hash) => self.user(id),
            _ => Ok(None),
        }
    }

    // ---------- invites ----------

    /// Create an invite. Returns the invite and its code; the code is not stored and can't be
    /// shown again.
    pub fn create_invite(
        &self,
        created_by: i64,
        permissions: &BTreeSet<String>,
        servers: Option<&BTreeSet<String>>,
        now: i64,
        ttl_secs: i64,
    ) -> Result<(Invite, String)> {
        let code = random_token()?; // 32 bytes from the OS random generator = 256 bits
        let id = sha256_hex(&code);
        self.db().execute(
            "INSERT INTO invites (code_hash, permissions, servers, created_by, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                serde_json::to_string(permissions).expect("strings serialise"),
                servers.map(|s| serde_json::to_string(s).expect("strings serialise")),
                created_by,
                now,
                now + ttl_secs
            ],
        )?;
        let invite = self
            .invite(&code, now)?
            .ok_or_else(|| StoreError::Invalid("invite vanished".into()))?;
        Ok((invite, code))
    }

    /// A usable invite for this code: exists, not used, not expired.
    pub fn invite(&self, code: &str, now: i64) -> Result<Option<Invite>> {
        if code.len() != 64 {
            return Ok(None);
        }
        Ok(self
            .db()
            .query_row(
                &format!(
                    "SELECT {INVITE_COLUMNS} FROM invites WHERE code_hash = ?1 AND used_at IS NULL AND expires_at >= ?2"
                ),
                params![sha256_hex(code), now],
                invite_from_row,
            )
            .optional()?)
    }

    /// Open (unused, unexpired) invites, newest first.
    pub fn open_invites(&self, now: i64) -> Result<Vec<Invite>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE used_at IS NULL AND expires_at >= ?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([now], invite_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cancel an open invite by its id (the hash, not the code).
    pub fn revoke_invite(&self, id: &str) -> Result<()> {
        self.db()
            .execute("DELETE FROM invites WHERE code_hash = ?1 AND used_at IS NULL", [id])?;
        Ok(())
    }

    /// Create an account from an invite. The invite is used up even if two people race for it:
    /// only the first redemption succeeds.
    pub fn redeem_invite(&self, code: &str, username: &str, password: &str, now: i64) -> Result<User> {
        let invite = self
            .invite(code, now)?
            .ok_or_else(|| StoreError::Invalid("this invite link is no longer valid".into()))?;
        let claimed = self.db().execute(
            "UPDATE invites SET used_at = ?1 WHERE code_hash = ?2 AND used_at IS NULL AND expires_at >= ?1",
            params![now, invite.id],
        )?;
        if claimed != 1 {
            return Err(StoreError::Invalid("this invite link is no longer valid".into()));
        }
        match self.create_user(
            username,
            password,
            false,
            &invite.permissions,
            invite.servers.as_ref(),
            now,
        ) {
            Ok(user) => {
                self.db().execute(
                    "UPDATE invites SET used_by = ?1 WHERE code_hash = ?2",
                    params![user.id, invite.id],
                )?;
                Ok(user)
            }
            Err(e) => {
                // Let them try again with a different name or password.
                self.db()
                    .execute("UPDATE invites SET used_at = NULL WHERE code_hash = ?1", [invite.id])?;
                Err(e)
            }
        }
    }

    // ---------- blocked addresses ----------

    /// The block on `ip`, if it is blocked right now.
    pub fn active_block(&self, ip: &str, now: i64) -> Result<Option<IpBlock>> {
        let b = self
            .db()
            .query_row(
                "SELECT ip, failures, strikes, blocked_until, permanent, last_failure FROM ip_blocks WHERE ip = ?1",
                [ip],
                block_from_row,
            )
            .optional()?;
        Ok(b.filter(|b| b.is_blocked(now)))
    }

    /// Record a wrong invite code from `ip`. Returns the block if this guess triggered one.
    pub fn record_bad_code(&self, ip: &str, now: i64) -> Result<Option<IpBlock>> {
        self.record_bad_code_with(ip, now, random_threshold)
    }

    fn record_bad_code_with(&self, ip: &str, now: i64, threshold: impl Fn() -> u32) -> Result<Option<IpBlock>> {
        let db = self.db();
        let row: Option<(u32, u32, u32, Option<i64>, bool, i64)> = db
            .query_row(
                "SELECT failures, threshold, strikes, blocked_until, permanent, last_failure FROM ip_blocks WHERE ip = ?1",
                [ip],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0, r.get(5)?)),
            )
            .optional()?;
        let (mut failures, mut limit, mut strikes, mut until, permanent, last) =
            row.unwrap_or((0, threshold(), 0, None, false, now));
        if permanent || until.is_some_and(|t| t > now) {
            return Ok(None); // already blocked
        }
        if now - last > FAILURE_MEMORY {
            failures = 0;
        }
        failures += 1;
        let mut blocked = false;
        let mut permanent_now = false;
        if failures >= limit {
            blocked = true;
            strikes += 1;
            match BLOCK_STEPS.get(strikes as usize - 1) {
                Some(secs) => until = Some(now + secs),
                None => {
                    permanent_now = true;
                    until = None;
                }
            }
            failures = 0;
            limit = threshold();
        }
        db.execute(
            "INSERT INTO ip_blocks (ip, failures, threshold, strikes, blocked_until, permanent, last_failure)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(ip) DO UPDATE SET failures = ?2, threshold = ?3, strikes = ?4, blocked_until = ?5,
                 permanent = ?6, last_failure = ?7",
            params![ip, failures, limit, strikes, until, permanent_now as i64, now],
        )?;
        Ok(blocked.then(|| IpBlock {
            ip: ip.to_string(),
            failures,
            strikes,
            blocked_until: until,
            permanent: permanent_now,
            last_failure: now,
        }))
    }

    /// Every address with at least one block, most recent first.
    pub fn blocks(&self) -> Result<Vec<IpBlock>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT ip, failures, strikes, blocked_until, permanent, last_failure FROM ip_blocks
             WHERE strikes > 0 ORDER BY last_failure DESC",
        )?;
        let rows = stmt.query_map([], block_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Lift the block on `ip` and forget its history. Returns whether anything was removed.
    pub fn unblock(&self, ip: &str) -> Result<bool> {
        Ok(self.db().execute("DELETE FROM ip_blocks WHERE ip = ?1", [ip])? > 0)
    }

    // ---------- sessions ----------

    pub fn create_session(&self, user_id: i64, now: i64, ttl_secs: i64) -> Result<String> {
        let token = random_token()?;
        let db = self.db();
        db.execute("DELETE FROM sessions WHERE expires_at < ?1", [now])?;
        db.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![token, user_id, now + ttl_secs],
        )?;
        Ok(token)
    }

    pub fn session_user(&self, token: &str, now: i64) -> Result<Option<User>> {
        let id: Option<i64> = self
            .db()
            .query_row(
                "SELECT user_id FROM sessions WHERE token = ?1 AND expires_at >= ?2",
                params![token, now],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.user(id),
            None => Ok(None),
        }
    }

    pub fn delete_session(&self, token: &str) -> Result<()> {
        self.db().execute("DELETE FROM sessions WHERE token = ?1", [token])?;
        Ok(())
    }

    // ---------- status history ----------

    pub fn record(&self, server_id: &str, status: &ServerStatus) -> Result<()> {
        self.db().execute(
            "INSERT INTO status_history (server_id, at, state, players, max_players) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                server_id,
                status.observed_at_unix,
                state_str(status.state),
                status.players_online,
                status.players_max
            ],
        )?;
        Ok(())
    }

    /// Samples for one server since `since` (unix seconds), oldest first.
    pub fn history(&self, server_id: &str, since: i64) -> Result<Vec<Sample>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT at, state, players, max_players FROM status_history
             WHERE server_id = ?1 AND at >= ?2 ORDER BY at",
        )?;
        let rows = stmt.query_map(params![server_id, since], |r| {
            Ok(Sample {
                at: r.get(0)?,
                state: state_from(&r.get::<_, String>(1)?),
                players: r.get(2)?,
                max_players: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Give freed pages back to the file system and checkpoint the WAL. Cheap; run after pruning.
    pub fn compact(&self) {
        let _ = self
            .db()
            .execute_batch("PRAGMA incremental_vacuum(2000); PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// Delete samples older than `before`.
    pub fn prune_history(&self, before: i64) -> Result<usize> {
        Ok(self
            .db()
            .execute("DELETE FROM status_history WHERE at < ?1", [before])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn superadmin_and_sub_admin_access() {
        let s = Store::open_in_memory().unwrap();
        assert!(!s.has_superadmin().unwrap());
        let root = s.set_superadmin("admin", "correct horse", 0).unwrap();
        assert!(root.is_superadmin && root.can("anything") && root.sees_server("x"));
        assert!(s.has_superadmin().unwrap());

        let only = perms(&["athens"]);
        let mod_ = s
            .create_user("mod", "password1", false, &perms(&["servers.view"]), Some(&only), 0)
            .unwrap();
        assert!(mod_.can("servers.view"));
        assert!(!mod_.can("logs.view"));
        assert!(mod_.sees_server("athens") && !mod_.sees_server("pz"));

        s.set_access(mod_.id, &perms(&["servers.view", "logs.view"]), None)
            .unwrap();
        let mod_ = s.user(mod_.id).unwrap().unwrap();
        assert!(mod_.can("logs.view") && mod_.sees_server("pz"));

        assert!(
            s.create_user("MOD", "password1", false, &perms(&[]), None, 0).is_err(),
            "names are case-insensitive"
        );
        assert!(s.create_user("x y", "password1", false, &perms(&[]), None, 0).is_err());
        assert!(s.create_user("short", "1234", false, &perms(&[]), None, 0).is_err());

        s.delete_user(root.id).unwrap();
        assert!(s.has_superadmin().unwrap(), "superadmin can't be deleted");
    }

    #[test]
    fn invites_are_single_use_and_expire() {
        let s = Store::open_in_memory().unwrap();
        let root = s.set_superadmin("admin", "correct horse", 0).unwrap();
        let only = perms(&["athens"]);
        let (inv, code) = s
            .create_invite(root.id, &perms(&["servers.view"]), Some(&only), 100, 60)
            .unwrap();
        assert_eq!(code.len(), 64, "256-bit code");
        assert_ne!(inv.id, code, "only the hash is stored");
        assert_eq!(s.open_invites(100).unwrap().len(), 1);
        assert!(
            s.invite(&inv.id, 100).unwrap().is_none(),
            "the stored hash is not a usable code"
        );

        // A bad password doesn't burn the invite.
        assert!(s.redeem_invite(&code, "newbie", "short", 120).is_err());
        let u = s.redeem_invite(&code, "newbie", "long enough", 120).unwrap();
        assert!(u.can("servers.view") && u.sees_server("athens") && !u.sees_server("pz"));
        assert!(
            s.redeem_invite(&code, "second", "long enough", 121).is_err(),
            "single use"
        );

        let (_, late) = s.create_invite(root.id, &perms(&[]), None, 100, 60).unwrap();
        assert!(s.invite(&late, 161).unwrap().is_none(), "expired");
        assert!(s.redeem_invite(&late, "late", "long enough", 161).is_err());
        assert!(s.redeem_invite("not-a-code", "x1", "long enough", 100).is_err());

        let (revoked, revoked_code) = s.create_invite(root.id, &perms(&[]), None, 100, 60).unwrap();
        s.revoke_invite(&revoked.id).unwrap();
        assert!(s.invite(&revoked_code, 100).unwrap().is_none());
    }

    #[test]
    fn bad_codes_escalate_blocks() {
        let s = Store::open_in_memory().unwrap();
        let ip = "203.0.113.9";
        let mut now = 1_000;
        let mut lengths = Vec::new();
        for _ in 0..5 {
            // Three wrong guesses per round (fixed threshold for the test).
            assert!(s.record_bad_code_with(ip, now, || 3).unwrap().is_none());
            assert!(s.record_bad_code_with(ip, now, || 3).unwrap().is_none());
            let b = s.record_bad_code_with(ip, now, || 3).unwrap().expect("blocked");
            assert!(s.active_block(ip, now).unwrap().is_some());
            assert!(
                s.record_bad_code_with(ip, now, || 3).unwrap().is_none(),
                "no stacking while blocked"
            );
            match b.blocked_until {
                Some(t) => {
                    lengths.push(t - now);
                    now = t + 1; // wait it out
                    assert!(s.active_block(ip, now).unwrap().is_none());
                }
                None => {
                    assert!(b.permanent);
                    lengths.push(-1);
                    now += 10 * 365 * 86_400;
                }
            }
        }
        assert_eq!(lengths, [1800, 10_800, 604_800, 2_592_000, -1]);
        assert!(
            s.active_block(ip, now).unwrap().is_some(),
            "permanent until an admin acts"
        );
        assert_eq!(s.blocks().unwrap().len(), 1);
        assert!(s.unblock(ip).unwrap());
        assert!(s.active_block(ip, now).unwrap().is_none());
    }

    #[test]
    fn random_threshold_stays_in_range() {
        for _ in 0..200 {
            assert!((2..=5).contains(&random_threshold()));
        }
    }

    #[test]
    fn logins_and_sessions() {
        let s = Store::open_in_memory().unwrap();
        let u = s.set_superadmin("admin", "correct horse", 0).unwrap();
        assert!(s.verify_login("admin", "wrong pass").unwrap().is_none());
        assert_eq!(
            s.verify_login("ADMIN", "correct horse").unwrap().map(|u| u.id),
            Some(u.id)
        );

        let t = s.create_session(u.id, 100, 60).unwrap();
        assert_eq!(s.session_user(&t, 150).unwrap().map(|u| u.id), Some(u.id));
        assert!(s.session_user(&t, 161).unwrap().is_none(), "expired");
        let t2 = s.create_session(u.id, 100, 60).unwrap();
        s.set_password(u.id, "new password").unwrap();
        assert!(s.session_user(&t2, 120).unwrap().is_none(), "password change signs out");
    }

    #[test]
    fn history_round_trip_and_prune() {
        let s = Store::open_in_memory().unwrap();
        let mut st = ServerStatus::new(ServerState::Online, 10);
        st.players_online = Some(3);
        st.players_max = Some(30);
        s.record("athens", &st).unwrap();
        s.record("athens", &ServerStatus::offline(20)).unwrap();
        let h = s.history("athens", 0).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].players, Some(3));
        assert_eq!(h[1].state, ServerState::Offline);
        assert_eq!(s.prune_history(15).unwrap(), 1);
    }
}
