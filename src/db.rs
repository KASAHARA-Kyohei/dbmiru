use std::{
    sync::mpsc::{self, Sender as BlockingSender},
    thread,
    time::{Duration, Instant},
};

use async_channel::Sender;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_postgres::{Client, NoTls, Row, types::Type};
use uuid::Uuid;

use crate::Result;
use crate::profiles::ConnectionProfile;

pub const ROW_LIMIT: usize = 1000;

pub enum DbEvent {
    Connected(DbSessionHandle),
    ConnectionFailed(String),
    ConnectionClosed(Option<String>),
    QueryFinished(QueryResult),
    QueryFailed(String),
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub duration: Duration,
    pub truncated: bool,
}

pub struct DbSessionHandle {
    commands: UnboundedSender<DbCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl DbSessionHandle {
    fn new(commands: UnboundedSender<DbCommand>, join_handle: thread::JoinHandle<()>) -> Self {
        Self {
            commands,
            join_handle: Some(join_handle),
        }
    }

    pub fn execute(&self, sql: String) {
        let _ = self.commands.send(DbCommand::Execute {
            sql,
            limit: ROW_LIMIT,
        });
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(DbCommand::Disconnect);
    }
}

impl Drop for DbSessionHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(DbCommand::Disconnect);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

enum DbCommand {
    Execute { sql: String, limit: usize },
    Disconnect,
}

pub fn spawn_session(profile: ConnectionProfile, password: String, event_tx: Sender<DbEvent>) {
    let (ready_tx, ready_rx) = mpsc::channel::<UnboundedSender<DbCommand>>();
    let worker_event_tx = event_tx.clone();
    let handshake_event_tx = event_tx;
    let join_handle = thread::spawn({
        let failure_tx = handshake_event_tx.clone();
        move || {
            if let Err(err) = run_worker(profile, password, ready_tx, worker_event_tx) {
                let _ = failure_tx.send_blocking(DbEvent::ConnectionFailed(err.to_string()));
            }
        }
    });

    thread::spawn(move || match ready_rx.recv() {
        Ok(command_tx) => {
            let handle = DbSessionHandle::new(command_tx, join_handle);
            let _ = handshake_event_tx.send_blocking(DbEvent::Connected(handle));
        }
        Err(_) => {
            let _ = handshake_event_tx.send_blocking(DbEvent::ConnectionFailed(
                "Failed to initialize connection worker".into(),
            ));
            let _ = join_handle.join();
        }
    });
}

fn run_worker(
    profile: ConnectionProfile,
    password: String,
    ready_tx: BlockingSender<UnboundedSender<DbCommand>>,
    event_tx: Sender<DbEvent>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let mut config = tokio_postgres::Config::new();
        config.host(&profile.host);
        config.port(profile.port);
        config.user(&profile.username);
        config.dbname(&profile.database);
        config.password(password);

        let (client, connection) = match config.connect(NoTls).await {
            Ok(conn) => conn,
            Err(err) => {
                let _ = event_tx
                    .send(DbEvent::ConnectionFailed(format!("{}", err)))
                    .await;
                return Err(err.into());
            }
        };

        let (command_tx, mut command_rx) = unbounded_channel::<DbCommand>();
        if ready_tx.send(command_tx).is_err() {
            return Ok(());
        }

        let (connection_closed_tx, connection_closed_rx) =
            tokio::sync::oneshot::channel::<Option<String>>();
        let connection_events = event_tx.clone();
        tokio::spawn(async move {
            let outcome = connection.await;
            let reason = outcome.err().map(|err| err.to_string());
            let _ = connection_closed_tx.send(reason.clone());
            let _ = connection_events
                .send(DbEvent::ConnectionClosed(reason))
                .await;
        });

        process_commands(client, &mut command_rx, event_tx.clone()).await;
        let _ = connection_closed_rx.await;
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

async fn process_commands(
    client: Client,
    command_rx: &mut UnboundedReceiver<DbCommand>,
    event_tx: Sender<DbEvent>,
) {
    let mut client = client;
    while let Some(command) = command_rx.recv().await {
        match command {
            DbCommand::Execute { sql, limit } => {
                execute_query(&mut client, sql, limit, event_tx.clone()).await;
            }
            DbCommand::Disconnect => break,
        }
    }
}

async fn execute_query(client: &mut Client, sql: String, limit: usize, event_tx: Sender<DbEvent>) {
    let started = Instant::now();
    match client.query(sql.as_str(), &[]).await {
        Ok(rows) => {
            let (columns, data_rows) = convert_rows(&rows, limit);
            let payload = QueryResult {
                columns,
                rows: data_rows,
                row_count: rows.len(),
                duration: started.elapsed(),
                truncated: rows.len() > limit,
            };
            let _ = event_tx.send(DbEvent::QueryFinished(payload)).await;
        }
        Err(err) => {
            let _ = event_tx
                .send(DbEvent::QueryFailed(format!("{}", err)))
                .await;
        }
    }
}

fn convert_rows(rows: &[Row], limit: usize) -> (Vec<String>, Vec<Vec<String>>) {
    let columns = rows
        .first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|col| col.name().to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut rendered_rows = Vec::new();
    for row in rows.iter().take(limit) {
        rendered_rows.push(render_row(row));
    }
    (columns, rendered_rows)
}

fn render_row(row: &Row) -> Vec<String> {
    let mut values = Vec::with_capacity(row.len());
    for (idx, column) in row.columns().iter().enumerate() {
        values.push(render_cell(row, idx, column.type_()));
    }
    values
}

fn render_cell(row: &Row, idx: usize, ty: &Type) -> String {
    match *ty {
        Type::BOOL => format_optional(row.try_get::<_, Option<bool>>(idx)),
        Type::INT2 => format_optional(row.try_get::<_, Option<i16>>(idx)),
        Type::INT4 => format_optional(row.try_get::<_, Option<i32>>(idx)),
        Type::INT8 => format_optional(row.try_get::<_, Option<i64>>(idx)),
        Type::FLOAT4 => format_optional(row.try_get::<_, Option<f32>>(idx)),
        Type::FLOAT8 => format_optional(row.try_get::<_, Option<f64>>(idx)),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
            format_optional(row.try_get::<_, Option<String>>(idx))
        }
        Type::TIMESTAMP => format_optional(
            row.try_get::<_, Option<NaiveDateTime>>(idx)
                .map(|opt| opt.map(|dt| dt.to_string())),
        ),
        Type::TIMESTAMPTZ => format_optional(
            row.try_get::<_, Option<DateTime<Utc>>>(idx)
                .map(|opt| opt.map(|dt| dt.to_rfc3339())),
        ),
        Type::DATE => format_optional(
            row.try_get::<_, Option<NaiveDate>>(idx)
                .map(|opt| opt.map(|d| d.to_string())),
        ),
        Type::UUID => format_optional(
            row.try_get::<_, Option<Uuid>>(idx)
                .map(|opt| opt.map(|v| v.to_string())),
        ),
        Type::JSON | Type::JSONB => format_optional(
            row.try_get::<_, Option<serde_json::Value>>(idx)
                .map(|opt| opt.map(|value| value.to_string())),
        ),
        Type::BYTEA => format_optional(
            row.try_get::<_, Option<Vec<u8>>>(idx)
                .map(|opt| opt.map(|bytes| format_bytea(&bytes))),
        ),
        _ => format_optional(
            row.try_get::<_, Option<String>>(idx)
                .map(|opt| opt.or_else(|| Some("<unsupported>".into()))),
        ),
    }
}

fn format_optional<T, E>(value: std::result::Result<Option<T>, E>) -> String
where
    T: ToString,
{
    match value {
        Ok(Some(inner)) => inner.to_string(),
        Ok(None) => "NULL".into(),
        Err(_) => "<err>".into(),
    }
}

fn format_bytea(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push_str("\\x");
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", byte);
    }
    out
}
