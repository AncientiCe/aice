use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::auth::Role;
use crate::FacilitatorError;

/// Failed logins allowed before an account is locked.
pub(crate) const MAX_LOGIN_FAILURES: i64 = 5;
/// How long a locked account stays locked.
pub(crate) const LOCKOUT_MILLIS: i64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketStatus {
    Open,
    Acknowledged,
    Done,
    Escalated,
}

impl TicketStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Done => "done",
            Self::Escalated => "escalated",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "acknowledged" => Some(Self::Acknowledged),
            "done" => Some(Self::Done),
            "escalated" => Some(Self::Escalated),
            _ => None,
        }
    }

    /// `Done -> Open` is a reopen; the desk only offers it to supervisors.
    pub fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Open, Self::Acknowledged)
                | (Self::Open, Self::Done)
                | (Self::Open, Self::Escalated)
                | (Self::Acknowledged, Self::Done)
                | (Self::Acknowledged, Self::Escalated)
                | (Self::Escalated, Self::Acknowledged)
                | (Self::Escalated, Self::Done)
                | (Self::Done, Self::Open)
        )
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Ticket {
    pub id: String,
    pub pack: String,
    pub room: String,
    pub tool_name: String,
    pub arguments_json: String,
    pub status: String,
    pub detail: String,
    pub created_millis: i64,
}

/// Everything needed to create a ticket.
pub(crate) struct NewTicket<'a> {
    pub pack: &'a str,
    pub room: &'a str,
    pub tool_name: &'a str,
    pub arguments_json: &'a str,
    pub status: TicketStatus,
    pub detail: &'a str,
    /// Schedule the first alert now.
    pub arm_alert: bool,
}

/// A ticket whose next alert is due. `tier` is `None` before the first alert.
#[derive(Clone, Debug)]
pub(crate) struct DueAlert {
    pub ticket: Ticket,
    pub tier: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditEvent {
    pub id: i64,
    pub at_millis: i64,
    pub actor: String,
    pub action: String,
    pub ticket_id: Option<String>,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub(crate) struct StaffSession {
    pub username: String,
    pub role: Role,
    pub csrf: String,
}

/// Pods waiting for a supervisor before new enrolments are refused.
pub(crate) const MAX_PENDING_DEVICES: i64 = 256;

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub device_id: String,
    pub room: Option<String>,
    pub status: String,
    pub firmware: String,
    pub last_seen_millis: i64,
    pub created_millis: i64,
}

/// A pod identity the backend can trust.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub room: String,
    /// Memory Palace wing of the room's open stay; `None` means the room has
    /// no guest or resident and nothing may be remembered.
    #[serde(default)]
    pub memory_wing: Option<String>,
}

/// One guest's or resident's time in a room.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Stay {
    pub id: String,
    pub room: String,
    pub memory_wing: String,
    pub opened_millis: i64,
    pub closed_millis: Option<i64>,
    pub purge_after_millis: Option<i64>,
    pub purged_millis: Option<i64>,
    pub continued_from: Option<String>,
}

pub(crate) enum EnrollOutcome {
    Pending,
    Active { room: String, token: Option<String> },
    Revoked,
    NonceMismatch,
    TooManyPending,
}

pub(crate) enum LoginState {
    Unknown,
    Locked,
    Known { password_hash: String },
}

pub struct TicketStore {
    connection: Mutex<Connection>,
}

impl TicketStore {
    pub fn open(path: &std::path::Path) -> Result<Self, FacilitatorError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS tickets (
                id TEXT PRIMARY KEY,
                pack TEXT NOT NULL,
                room TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments_json TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS users (
                username TEXT PRIMARY KEY,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT PRIMARY KEY,
                username TEXT NOT NULL,
                csrf TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS login_failures (
                username TEXT PRIMARY KEY,
                failures INTEGER NOT NULL,
                locked_until INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at_millis INTEGER NOT NULL,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                ticket_id TEXT,
                from_status TEXT,
                to_status TEXT,
                detail TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS devices (
                device_id TEXT PRIMARY KEY,
                room TEXT,
                status TEXT NOT NULL,
                token_hash TEXT UNIQUE,
                pending_token TEXT,
                nonce_hash TEXT NOT NULL,
                firmware TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS stays (
                id TEXT PRIMARY KEY,
                room TEXT NOT NULL,
                memory_wing TEXT NOT NULL,
                opened_at INTEGER NOT NULL,
                closed_at INTEGER,
                purge_after INTEGER,
                purged_at INTEGER,
                continued_from TEXT
            );
            CREATE INDEX IF NOT EXISTS stays_room_open ON stays(room, closed_at);
            CREATE TRIGGER IF NOT EXISTS audit_events_no_update
                BEFORE UPDATE ON audit_events
                BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS audit_events_no_delete
                BEFORE DELETE ON audit_events
                BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;",
        )?;
        add_column_if_missing(&connection, "tickets", "alert_tier", "INTEGER")?;
        add_column_if_missing(&connection, "tickets", "next_alert_at", "INTEGER")?;
        connection.execute_batch(
            "CREATE INDEX IF NOT EXISTS tickets_next_alert ON tickets(next_alert_at);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, FacilitatorError> {
        self.connection
            .lock()
            .map_err(|_| FacilitatorError::LockPoisoned)
    }

    pub(crate) fn insert(&self, new: NewTicket<'_>) -> Result<Ticket, FacilitatorError> {
        let NewTicket {
            pack,
            room,
            tool_name,
            arguments_json,
            status,
            detail,
            arm_alert,
        } = new;
        let id = next_ticket_id();
        let now = unix_millis();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO tickets (id, pack, room, tool_name, arguments_json, status, detail, created_at, updated_at, alert_tier, next_alert_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
            params![
                id,
                pack,
                room,
                tool_name,
                arguments_json,
                status.as_str(),
                detail,
                now,
                now,
                arm_alert.then_some(now)
            ],
        )?;
        append_audit(
            &transaction,
            "voice",
            "ticket_created",
            Some(&id),
            None,
            Some(status.as_str()),
            detail,
        )?;
        transaction.commit()?;
        Ok(Ticket {
            id,
            pack: pack.to_string(),
            room: room.to_string(),
            tool_name: tool_name.to_string(),
            arguments_json: arguments_json.to_string(),
            status: status.as_str().to_string(),
            detail: detail.to_string(),
            created_millis: now,
        })
    }

    pub fn list(&self) -> Result<Vec<Ticket>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, pack, room, tool_name, arguments_json, status, detail, created_at
             FROM tickets ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([], ticket_from_row)?;
        let mut tickets = Vec::new();
        for row in rows {
            tickets.push(row?);
        }
        Ok(tickets)
    }

    /// Change a ticket's status and record who did it, in one transaction.
    pub fn set_status(
        &self,
        id: &str,
        next: TicketStatus,
        actor: &str,
    ) -> Result<Ticket, FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let current: String = transaction
            .query_row(
                "SELECT status FROM tickets WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    FacilitatorError::TicketNotFound(id.to_string())
                }
                other => FacilitatorError::Sqlite(other),
            })?;
        let current_status =
            TicketStatus::parse(&current).ok_or_else(|| FacilitatorError::InvalidTransition {
                from: current.clone(),
                to: next.as_str().to_string(),
            })?;
        if !current_status.can_become(next) {
            return Err(FacilitatorError::InvalidTransition {
                from: current,
                to: next.as_str().to_string(),
            });
        }
        let now = unix_millis();
        transaction.execute(
            "UPDATE tickets SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![next.as_str(), now, id],
        )?;
        match next {
            TicketStatus::Acknowledged | TicketStatus::Done => {
                transaction.execute(
                    "UPDATE tickets SET next_alert_at = NULL WHERE id = ?1",
                    params![id],
                )?;
            }
            // A reopened ticket is announced again from the first tier.
            TicketStatus::Open => {
                transaction.execute(
                    "UPDATE tickets SET next_alert_at = ?1, alert_tier = NULL WHERE id = ?2",
                    params![now, id],
                )?;
            }
            TicketStatus::Escalated => {}
        }
        append_audit(
            &transaction,
            actor,
            "ticket_status",
            Some(id),
            Some(current_status.as_str()),
            Some(next.as_str()),
            "",
        )?;
        let ticket = transaction.query_row(
            "SELECT id, pack, room, tool_name, arguments_json, status, detail, created_at FROM tickets WHERE id = ?1",
            params![id],
            ticket_from_row,
        )?;
        transaction.commit()?;
        Ok(ticket)
    }

    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditEvent>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, at_millis, actor, action, ticket_id, from_status, to_status, detail
             FROM audit_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], |row| {
            Ok(AuditEvent {
                id: row.get(0)?,
                at_millis: row.get(1)?,
                actor: row.get(2)?,
                action: row.get(3)?,
                ticket_id: row.get(4)?,
                from_status: row.get(5)?,
                to_status: row.get(6)?,
                detail: row.get(7)?,
            })
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub(crate) fn insert_user(
        &self,
        username: &str,
        password_hash: &str,
        role: Role,
    ) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO users (username, password_hash, role, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![username, password_hash, role.as_str(), unix_millis()],
        )?;
        if inserted == 0 {
            return Err(FacilitatorError::DuplicateUser(username.to_string()));
        }
        append_audit(
            &transaction,
            "admin",
            "user_added",
            None,
            None,
            None,
            &format!("{username} ({})", role.as_str()),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn remove_user(&self, username: &str) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let removed =
            transaction.execute("DELETE FROM users WHERE username = ?1", params![username])?;
        if removed == 0 {
            return Err(FacilitatorError::UnknownUser(username.to_string()));
        }
        transaction.execute(
            "DELETE FROM sessions WHERE username = ?1",
            params![username],
        )?;
        transaction.execute(
            "DELETE FROM login_failures WHERE username = ?1",
            params![username],
        )?;
        append_audit(
            &transaction,
            "admin",
            "user_removed",
            None,
            None,
            None,
            username,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn list_users(&self) -> Result<Vec<(String, Role)>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement =
            connection.prepare("SELECT username, role FROM users ORDER BY username ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut users = Vec::new();
        for row in rows {
            let (username, role) = row?;
            if let Some(role) = Role::parse(&role) {
                users.push((username, role));
            }
        }
        Ok(users)
    }

    pub(crate) fn has_users(&self) -> Result<bool, FacilitatorError> {
        let connection = self.lock()?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    pub(crate) fn login_state(&self, username: &str) -> Result<LoginState, FacilitatorError> {
        let connection = self.lock()?;
        let locked_until: Option<i64> = connection
            .query_row(
                "SELECT locked_until FROM login_failures WHERE username = ?1",
                params![username],
                |row| row.get(0),
            )
            .optional()?;
        if locked_until.is_some_and(|until| until > unix_millis()) {
            return Ok(LoginState::Locked);
        }
        let password_hash: Option<String> = connection
            .query_row(
                "SELECT password_hash FROM users WHERE username = ?1",
                params![username],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match password_hash {
            Some(password_hash) => LoginState::Known { password_hash },
            None => LoginState::Unknown,
        })
    }

    /// Count a failed login; returns true when this failure locked the account.
    pub(crate) fn record_login_failure(&self, username: &str) -> Result<bool, FacilitatorError> {
        let connection = self.lock()?;
        let now = unix_millis();
        let failures: i64 = connection
            .query_row(
                "SELECT failures FROM login_failures WHERE username = ?1",
                params![username],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0)
            + 1;
        let (failures, locked_until) = if failures >= MAX_LOGIN_FAILURES {
            (0, now + LOCKOUT_MILLIS)
        } else {
            (failures, 0)
        };
        connection.execute(
            "INSERT INTO login_failures (username, failures, locked_until) VALUES (?1, ?2, ?3)
             ON CONFLICT(username) DO UPDATE SET failures = ?2, locked_until = ?3",
            params![username, failures, locked_until],
        )?;
        let locked = locked_until > 0;
        if locked {
            append_audit(
                &connection,
                username,
                "account_locked",
                None,
                None,
                None,
                "too many failed logins",
            )?;
        }
        Ok(locked)
    }

    pub(crate) fn clear_login_failures(&self, username: &str) -> Result<(), FacilitatorError> {
        let connection = self.lock()?;
        connection.execute(
            "DELETE FROM login_failures WHERE username = ?1",
            params![username],
        )?;
        Ok(())
    }

    pub(crate) fn create_session(
        &self,
        token_hash: &str,
        username: &str,
        csrf: &str,
        expires_at: i64,
    ) -> Result<(), FacilitatorError> {
        let connection = self.lock()?;
        connection.execute(
            "DELETE FROM sessions WHERE expires_at <= ?1",
            params![unix_millis()],
        )?;
        connection.execute(
            "INSERT INTO sessions (token_hash, username, csrf, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, username, csrf, expires_at],
        )?;
        append_audit(&connection, username, "login", None, None, None, "")?;
        Ok(())
    }

    pub(crate) fn session(
        &self,
        token_hash: &str,
    ) -> Result<Option<StaffSession>, FacilitatorError> {
        let connection = self.lock()?;
        let row: Option<(String, String, String)> = connection
            .query_row(
                "SELECT s.username, u.role, s.csrf FROM sessions s
                 JOIN users u ON u.username = s.username
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![token_hash, unix_millis()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(username, role, csrf)| {
            Role::parse(&role).map(|role| StaffSession {
                username,
                role,
                csrf,
            })
        }))
    }

    pub(crate) fn delete_session(
        &self,
        token_hash: &str,
        username: &str,
    ) -> Result<(), FacilitatorError> {
        let connection = self.lock()?;
        connection.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        append_audit(&connection, username, "logout", None, None, None, "")?;
        Ok(())
    }
}

impl TicketStore {
    pub(crate) fn enroll_device(
        &self,
        device_id: &str,
        nonce_hash: &str,
        firmware: &str,
    ) -> Result<EnrollOutcome, FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let now = unix_millis();
        let existing: Option<(String, Option<String>, Option<String>, String)> = transaction
            .query_row(
                "SELECT status, room, pending_token, nonce_hash FROM devices WHERE device_id = ?1",
                params![device_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let outcome = match existing {
            None => {
                let pending: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM devices WHERE status = 'pending'",
                    [],
                    |row| row.get(0),
                )?;
                if pending >= MAX_PENDING_DEVICES {
                    return Ok(EnrollOutcome::TooManyPending);
                }
                transaction.execute(
                    "INSERT INTO devices (device_id, room, status, token_hash, pending_token, nonce_hash, firmware, last_seen, created_at)
                     VALUES (?1, NULL, 'pending', NULL, NULL, ?2, ?3, ?4, ?4)",
                    params![device_id, nonce_hash, firmware, now],
                )?;
                append_audit(
                    &transaction,
                    device_id,
                    "device_enrolled",
                    None,
                    None,
                    None,
                    firmware,
                )?;
                EnrollOutcome::Pending
            }
            Some((_, _, _, stored_nonce))
                if !crate::auth::constant_time_eq(
                    stored_nonce.as_bytes(),
                    nonce_hash.as_bytes(),
                ) =>
            {
                append_audit(
                    &transaction,
                    device_id,
                    "device_nonce_mismatch",
                    None,
                    None,
                    None,
                    "",
                )?;
                EnrollOutcome::NonceMismatch
            }
            Some((status, room, pending_token, _)) => {
                transaction.execute(
                    "UPDATE devices SET firmware = ?1, last_seen = ?2, pending_token = NULL WHERE device_id = ?3",
                    params![firmware, now, device_id],
                )?;
                match (status.as_str(), room) {
                    ("active", Some(room)) => {
                        if pending_token.is_some() {
                            append_audit(
                                &transaction,
                                device_id,
                                "device_token_delivered",
                                None,
                                None,
                                None,
                                &room,
                            )?;
                        }
                        EnrollOutcome::Active {
                            room,
                            token: pending_token,
                        }
                    }
                    ("revoked", _) => EnrollOutcome::Revoked,
                    _ => EnrollOutcome::Pending,
                }
            }
        };
        transaction.commit()?;
        Ok(outcome)
    }

    pub(crate) fn list_devices(&self) -> Result<Vec<Device>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT device_id, room, status, firmware, last_seen, created_at
             FROM devices ORDER BY status ASC, device_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Device {
                device_id: row.get(0)?,
                room: row.get(1)?,
                status: row.get(2)?,
                firmware: row.get(3)?,
                last_seen_millis: row.get(4)?,
                created_millis: row.get(5)?,
            })
        })?;
        let mut devices = Vec::new();
        for row in rows {
            devices.push(row?);
        }
        Ok(devices)
    }

    /// Put a pod in a room. A pod without a live token gets a fresh one,
    /// held until the pod next enrols.
    pub(crate) fn assign_device(
        &self,
        device_id: &str,
        room: &str,
        actor: &str,
    ) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM devices WHERE device_id = ?1",
                params![device_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Err(FacilitatorError::UnknownDevice(device_id.to_string()));
        };
        if status == "active" {
            transaction.execute(
                "UPDATE devices SET room = ?1 WHERE device_id = ?2",
                params![room, device_id],
            )?;
        } else {
            let token = crate::auth::random_token();
            transaction.execute(
                "UPDATE devices SET room = ?1, status = 'active', token_hash = ?2, pending_token = ?3
                 WHERE device_id = ?4",
                params![room, crate::auth::token_digest(&token), token, device_id],
            )?;
        }
        append_audit(
            &transaction,
            actor,
            "device_assigned",
            None,
            None,
            None,
            &format!("{device_id} -> {room}"),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn revoke_device(
        &self,
        device_id: &str,
        actor: &str,
    ) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE devices SET status = 'revoked', token_hash = NULL, pending_token = NULL
             WHERE device_id = ?1",
            params![device_id],
        )?;
        if changed == 0 {
            return Err(FacilitatorError::UnknownDevice(device_id.to_string()));
        }
        append_audit(
            &transaction,
            actor,
            "device_revoked",
            None,
            None,
            None,
            device_id,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Resolve an active device token and mark the device as seen.
    pub(crate) fn verify_device(
        &self,
        token_hash: &str,
    ) -> Result<Option<DeviceIdentity>, FacilitatorError> {
        let connection = self.lock()?;
        let identity: Option<DeviceIdentity> = connection
            .query_row(
                "SELECT d.device_id, d.room,
                        (SELECT s.memory_wing FROM stays s
                         WHERE s.room = d.room AND s.closed_at IS NULL
                         ORDER BY s.opened_at DESC LIMIT 1)
                 FROM devices d
                 WHERE d.token_hash = ?1 AND d.status = 'active' AND d.room IS NOT NULL",
                params![token_hash],
                |row| {
                    Ok(DeviceIdentity {
                        device_id: row.get(0)?,
                        room: row.get(1)?,
                        memory_wing: row.get(2)?,
                    })
                },
            )
            .optional()?;
        if let Some(identity) = &identity {
            connection.execute(
                "UPDATE devices SET last_seen = ?1 WHERE device_id = ?2",
                params![unix_millis(), identity.device_id],
            )?;
        }
        Ok(identity)
    }
}

impl TicketStore {
    /// Check a guest or resident into `room`. `continue_from` reuses the memory
    /// of an earlier stay that has not been (and is not about to be) purged.
    pub(crate) fn open_stay(
        &self,
        room: &str,
        continue_from: Option<&str>,
        actor: &str,
    ) -> Result<Stay, FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let now = unix_millis();
        let open: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM stays WHERE room = ?1 AND closed_at IS NULL",
            params![room],
            |row| row.get(0),
        )?;
        if open > 0 {
            return Err(FacilitatorError::StayConflict(format!(
                "room {room} already has an open stay"
            )));
        }
        let id = format!("s{}-{}", now, &crate::auth::random_token()[..8]);
        let memory_wing = match continue_from {
            Some(previous) => {
                let earlier = transaction
                    .query_row(
                        &format!("{STAY_COLUMNS} WHERE id = ?1"),
                        params![previous],
                        stay_from_row,
                    )
                    .optional()?;
                let Some(earlier) = earlier else {
                    return Err(FacilitatorError::UnknownStay(previous.to_string()));
                };
                let reusable = earlier.closed_millis.is_some()
                    && earlier.purged_millis.is_none()
                    && earlier.purge_after_millis.is_none_or(|due| due > now);
                if !reusable {
                    return Err(FacilitatorError::StayConflict(format!(
                        "stay {previous} is open, purged, or due for purge"
                    )));
                }
                // The wing lives on in the new stay, so the old stay must not purge it.
                transaction.execute(
                    "UPDATE stays SET purge_after = NULL WHERE id = ?1",
                    params![previous],
                )?;
                earlier.memory_wing
            }
            None => format!("stay-{id}"),
        };
        transaction.execute(
            "INSERT INTO stays (id, room, memory_wing, opened_at, closed_at, purge_after, purged_at, continued_from)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, ?5)",
            params![id, room, memory_wing, now, continue_from],
        )?;
        append_audit(
            &transaction,
            actor,
            "stay_opened",
            None,
            None,
            None,
            &format!("{id} room {room}"),
        )?;
        let stay = transaction.query_row(
            &format!("{STAY_COLUMNS} WHERE id = ?1"),
            params![id],
            stay_from_row,
        )?;
        transaction.commit()?;
        Ok(stay)
    }

    pub(crate) fn close_stay(
        &self,
        id: &str,
        retention: crate::MemoryRetention,
        actor: &str,
    ) -> Result<Stay, FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let now = unix_millis();
        let changed = transaction.execute(
            "UPDATE stays SET closed_at = ?1, purge_after = ?2 WHERE id = ?3 AND closed_at IS NULL",
            params![now, retention.purge_after(now), id],
        )?;
        if changed == 0 {
            return Err(FacilitatorError::UnknownStay(id.to_string()));
        }
        append_audit(
            &transaction,
            actor,
            "stay_closed",
            None,
            None,
            None,
            &format!("{id} retention {}", retention.label()),
        )?;
        let stay = transaction.query_row(
            &format!("{STAY_COLUMNS} WHERE id = ?1"),
            params![id],
            stay_from_row,
        )?;
        transaction.commit()?;
        Ok(stay)
    }

    pub(crate) fn list_stays(&self) -> Result<Vec<Stay>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "{STAY_COLUMNS} ORDER BY closed_at IS NOT NULL, opened_at DESC LIMIT 200"
        ))?;
        let rows = statement.query_map([], stay_from_row)?;
        let mut stays = Vec::new();
        for row in rows {
            stays.push(row?);
        }
        Ok(stays)
    }

    /// Closed stays whose memory wing should now be deleted.
    pub(crate) fn purge_due(&self) -> Result<Vec<Stay>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "{STAY_COLUMNS} WHERE purged_at IS NULL AND purge_after IS NOT NULL AND purge_after <= ?1
             ORDER BY purge_after ASC"
        ))?;
        let rows = statement.query_map(params![unix_millis()], stay_from_row)?;
        let mut stays = Vec::new();
        for row in rows {
            stays.push(row?);
        }
        Ok(stays)
    }

    pub(crate) fn mark_stay_purged(&self, id: &str) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE stays SET purged_at = ?1 WHERE id = ?2 AND purge_after IS NOT NULL AND purged_at IS NULL",
            params![unix_millis(), id],
        )?;
        if changed == 0 {
            return Err(FacilitatorError::UnknownStay(id.to_string()));
        }
        append_audit(
            &transaction,
            "voice",
            "stay_memory_purged",
            None,
            None,
            None,
            id,
        )?;
        transaction.commit()?;
        Ok(())
    }
}

const STAY_COLUMNS: &str =
    "SELECT id, room, memory_wing, opened_at, closed_at, purge_after, purged_at, continued_from FROM stays";

fn stay_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stay> {
    Ok(Stay {
        id: row.get(0)?,
        room: row.get(1)?,
        memory_wing: row.get(2)?,
        opened_millis: row.get(3)?,
        closed_millis: row.get(4)?,
        purge_after_millis: row.get(5)?,
        purged_millis: row.get(6)?,
        continued_from: row.get(7)?,
    })
}

fn ticket_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Ticket> {
    Ok(Ticket {
        id: row.get(0)?,
        pack: row.get(1)?,
        room: row.get(2)?,
        tool_name: row.get(3)?,
        arguments_json: row.get(4)?,
        status: row.get(5)?,
        detail: row.get(6)?,
        created_millis: row.get(7)?,
    })
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), FacilitatorError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(());
        }
    }
    connection.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition};"
    ))?;
    Ok(())
}

impl TicketStore {
    /// Open or escalated tickets whose next alert is due.
    pub(crate) fn due_alerts(&self) -> Result<Vec<DueAlert>, FacilitatorError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, pack, room, tool_name, arguments_json, status, detail, created_at, alert_tier
             FROM tickets
             WHERE next_alert_at IS NOT NULL AND next_alert_at <= ?1
               AND status IN ('open', 'escalated')
             ORDER BY next_alert_at ASC LIMIT 100",
        )?;
        let rows = statement.query_map(params![unix_millis()], |row| {
            let tier: Option<i64> = row.get(8)?;
            Ok(DueAlert {
                ticket: ticket_from_row(row)?,
                tier: tier.and_then(|tier| usize::try_from(tier).ok()),
            })
        })?;
        let mut due = Vec::new();
        for row in rows {
            due.push(row?);
        }
        Ok(due)
    }

    pub(crate) fn disarm_alert(&self, id: &str) -> Result<(), FacilitatorError> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE tickets SET next_alert_at = NULL WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub(crate) fn rearm_alert(
        &self,
        id: &str,
        tier: Option<usize>,
        next_in: std::time::Duration,
    ) -> Result<(), FacilitatorError> {
        let connection = self.lock()?;
        let next_at = unix_millis() + i64::try_from(next_in.as_millis()).unwrap_or(i64::MAX / 2);
        let tier = tier.and_then(|tier| i64::try_from(tier).ok());
        // Only re-arm while the ticket still needs someone.
        connection.execute(
            "UPDATE tickets SET alert_tier = ?1, next_alert_at = ?2
             WHERE id = ?3 AND status IN ('open', 'escalated')",
            params![tier, next_at, id],
        )?;
        Ok(())
    }

    /// Nobody acknowledged in time: escalate an open ticket and audit the breach.
    pub(crate) fn record_sla_breach(&self, id: &str, tier: &str) -> Result<(), FacilitatorError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let escalated = transaction.execute(
            "UPDATE tickets SET status = 'escalated', updated_at = ?1 WHERE id = ?2 AND status = 'open'",
            params![unix_millis(), id],
        )?;
        if escalated > 0 {
            append_audit(
                &transaction,
                "sla",
                "ticket_status",
                Some(id),
                Some("open"),
                Some("escalated"),
                "not acknowledged in time",
            )?;
        }
        append_audit(
            &transaction,
            "sla",
            "sla_breach",
            Some(id),
            None,
            None,
            tier,
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn append_audit(
    connection: &Connection,
    actor: &str,
    action: &str,
    ticket_id: Option<&str>,
    from_status: Option<&str>,
    to_status: Option<&str>,
    detail: &str,
) -> Result<(), FacilitatorError> {
    connection.execute(
        "INSERT INTO audit_events (at_millis, actor, action, ticket_id, from_status, to_status, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            unix_millis(),
            actor,
            action,
            ticket_id,
            from_status,
            to_status,
            detail
        ],
    )?;
    core_observability::record_property_audit_event(action);
    Ok(())
}

fn next_ticket_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("t{}-{sequence}", unix_millis())
}

pub(crate) fn unix_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}
