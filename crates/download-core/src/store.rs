//! SQLite persistence for the in-memory task list.
//!
//! Path: `{config_dir}/download-tool/tasks.db` (same folder as settings).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::task::{DownloadTask, TaskId, TaskStatus};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    url TEXT NOT NULL UNIQUE,
    file_name TEXT NOT NULL,
    size INTEGER,
    referrer TEXT,
    user_agent TEXT,
    status TEXT NOT NULL,
    gid TEXT,
    error TEXT,
    completed_length INTEGER NOT NULL DEFAULT 0,
    download_speed INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
";

/// Opens and syncs download tasks to a local SQLite file.
pub struct TaskStore {
    conn: Mutex<Connection>,
}

impl TaskStore {
    /// `{config_dir}/download-tool/tasks.db`.
    pub fn file_in(config_dir: &Path) -> PathBuf {
        config_dir.join("download-tool").join("tasks.db")
    }

    pub fn config_file() -> Result<PathBuf, String> {
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| "could not resolve the OS config directory".to_string())?;
        Ok(Self::file_in(dirs.config_dir()))
    }

    pub fn open_user() -> Result<Self, String> {
        Self::open(&Self::config_file()?)
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create tasks dir {}: {err}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .map_err(|err| format!("open tasks db {}: {err}", path.display()))?;
        conn.execute_batch(SCHEMA)
            .map_err(|err| format!("init tasks schema: {err}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn load_all(&self) -> Result<Vec<DownloadTask>, String> {
        let conn = self.conn.lock().map_err(|_| "tasks db lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT id, url, file_name, size, referrer, user_agent, status, gid, error,
                        completed_length, download_speed, created_at
                 FROM tasks
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|err| format!("prepare load tasks: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(DownloadTask {
                    id: TaskId(row.get(0)?),
                    url: row.get(1)?,
                    file_name: row.get(2)?,
                    size: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    referrer: row.get(4)?,
                    user_agent: row.get(5)?,
                    status: parse_status(&row.get::<_, String>(6)?),
                    gid: row.get(7)?,
                    error: row.get(8)?,
                    completed_length: row.get::<_, i64>(9)? as u64,
                    download_speed: row.get::<_, i64>(10)? as u64,
                    created_at: row.get::<_, i64>(11)? as u64,
                })
            })
            .map_err(|err| format!("query tasks: {err}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| format!("read task row: {err}"))?);
        }
        Ok(out)
    }

    /// Replaces the on-disk snapshot with `tasks` (upsert + delete missing).
    pub fn sync_snapshot(&self, tasks: &[DownloadTask]) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|_| "tasks db lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("begin tasks sync: {err}"))?;
        {
            let mut keep = std::collections::HashSet::new();
            for task in tasks {
                keep.insert(task.id.0.clone());
                tx.execute(
                    "INSERT INTO tasks (
                        id, url, file_name, size, referrer, user_agent, status, gid, error,
                        completed_length, download_speed, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(id) DO UPDATE SET
                        url = excluded.url,
                        file_name = excluded.file_name,
                        size = excluded.size,
                        referrer = excluded.referrer,
                        user_agent = excluded.user_agent,
                        status = excluded.status,
                        gid = excluded.gid,
                        error = excluded.error,
                        completed_length = excluded.completed_length,
                        download_speed = excluded.download_speed,
                        created_at = excluded.created_at",
                    params![
                        task.id.0,
                        task.url,
                        task.file_name,
                        task.size.map(|v| v as i64),
                        task.referrer,
                        task.user_agent,
                        status_str(task.status),
                        task.gid,
                        task.error,
                        task.completed_length as i64,
                        task.download_speed as i64,
                        task.created_at as i64,
                    ],
                )
                .map_err(|err| format!("upsert task {}: {err}", task.id.0))?;
            }
            let existing: Vec<String> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM tasks")
                    .map_err(|err| format!("list task ids: {err}"))?;
                let ids = stmt
                    .query_map([], |row| row.get(0))
                    .map_err(|err| format!("query task ids: {err}"))?;
                let mut out = Vec::new();
                for id in ids {
                    out.push(id.map_err(|err| format!("read task id: {err}"))?);
                }
                out
            };
            for id in existing {
                if !keep.contains(&id) {
                    tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])
                        .map_err(|err| format!("delete task {id}: {err}"))?;
                }
            }
        }
        tx.commit()
            .map_err(|err| format!("commit tasks sync: {err}"))?;
        Ok(())
    }

    pub fn count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|_| "tasks db lock poisoned")?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .optional()
            .map_err(|err| format!("count tasks: {err}"))?
            .unwrap_or(0);
        Ok(n as usize)
    }
}

/// After process restart aria2 GIDs are invalid; unfinished work goes back to the queue.
pub fn prepare_loaded_task(mut task: DownloadTask) -> DownloadTask {
    match task.status {
        TaskStatus::Active | TaskStatus::Paused | TaskStatus::Queued => {
            task.status = TaskStatus::Queued;
            task.gid = None;
            task.download_speed = 0;
            task.error = None;
        }
        TaskStatus::Complete | TaskStatus::Error => {
            task.gid = None;
            task.download_speed = 0;
        }
    }
    task
}

fn status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "queued",
        TaskStatus::Active => "active",
        TaskStatus::Paused => "paused",
        TaskStatus::Complete => "complete",
        TaskStatus::Error => "error",
    }
}

fn parse_status(raw: &str) -> TaskStatus {
    match raw {
        "active" => TaskStatus::Active,
        "paused" => TaskStatus::Paused,
        "complete" => TaskStatus::Complete,
        "error" => TaskStatus::Error,
        _ => TaskStatus::Queued,
    }
}
