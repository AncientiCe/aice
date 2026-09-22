use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::FacilitatorError;

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
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn insert(
        &self,
        pack: &str,
        room: &str,
        tool_name: &str,
        arguments_json: &str,
        status: TicketStatus,
        detail: &str,
    ) -> Result<Ticket, FacilitatorError> {
        let id = next_ticket_id();
        let now = unix_millis();
        let connection = self
            .connection
            .lock()
            .map_err(|_| FacilitatorError::LockPoisoned)?;
        connection.execute(
            "INSERT INTO tickets (id, pack, room, tool_name, arguments_json, status, detail, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                pack,
                room,
                tool_name,
                arguments_json,
                status.as_str(),
                detail,
                now,
                now
            ],
        )?;
        Ok(Ticket {
            id,
            pack: pack.to_string(),
            room: room.to_string(),
            tool_name: tool_name.to_string(),
            arguments_json: arguments_json.to_string(),
            status: status.as_str().to_string(),
            detail: detail.to_string(),
        })
    }

    pub fn list(&self) -> Result<Vec<Ticket>, FacilitatorError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| FacilitatorError::LockPoisoned)?;
        let mut statement = connection.prepare(
            "SELECT id, pack, room, tool_name, arguments_json, status, detail
             FROM tickets ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Ticket {
                id: row.get(0)?,
                pack: row.get(1)?,
                room: row.get(2)?,
                tool_name: row.get(3)?,
                arguments_json: row.get(4)?,
                status: row.get(5)?,
                detail: row.get(6)?,
            })
        })?;
        let mut tickets = Vec::new();
        for row in rows {
            tickets.push(row?);
        }
        Ok(tickets)
    }

    pub fn set_status(&self, id: &str, next: TicketStatus) -> Result<Ticket, FacilitatorError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| FacilitatorError::LockPoisoned)?;
        let current: String = connection
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
        connection.execute(
            "UPDATE tickets SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![next.as_str(), now, id],
        )?;
        connection
            .query_row(
                "SELECT id, pack, room, tool_name, arguments_json, status, detail FROM tickets WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Ticket {
                        id: row.get(0)?,
                        pack: row.get(1)?,
                        room: row.get(2)?,
                        tool_name: row.get(3)?,
                        arguments_json: row.get(4)?,
                        status: row.get(5)?,
                        detail: row.get(6)?,
                    })
                },
            )
            .map_err(FacilitatorError::from)
    }
}

fn next_ticket_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("t{}-{sequence}", unix_millis())
}

fn unix_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}
