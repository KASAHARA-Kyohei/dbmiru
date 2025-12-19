use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use dbmiru_core::profiles::ConnectionProfile;
use tokio_postgres::{Client, NoTls, Row, types::Type};
use uuid::Uuid;

use crate::{
    ColumnMetadata, ConnectionClosedFuture, ConnectionError, DbAdapter, QueryResult, ROW_LIMIT,
    Result,
};

pub struct PostgresAdapter {
    profile: ConnectionProfile,
    password: String,
    client: Option<Client>,
    disconnecting: Arc<AtomicBool>,
}

impl PostgresAdapter {
    pub fn new(profile: ConnectionProfile, password: String) -> Self {
        Self {
            profile,
            password,
            client: None,
            disconnecting: Arc::new(AtomicBool::new(false)),
        }
    }

    fn client(&mut self) -> Result<&mut Client> {
        self.client
            .as_mut()
            .ok_or_else(|| anyhow!("Database client is not connected."))
    }
}

#[async_trait]
impl DbAdapter for PostgresAdapter {
    async fn connect(
        &mut self,
    ) -> std::result::Result<Option<ConnectionClosedFuture>, ConnectionError> {
        let mut config = tokio_postgres::Config::new();
        config.host(&self.profile.host);
        config.port(self.profile.port);
        config.user(&self.profile.username);
        config.dbname(&self.profile.database);
        config.password(&self.password);

        let (client, connection) = match config.connect(NoTls).await {
            Ok(conn) => conn,
            Err(err) => return Err(classify_connection_error(&err)),
        };
        let disconnecting = self.disconnecting.clone();
        let monitor = Box::pin(async move {
            let outcome = connection.await;
            if disconnecting.load(Ordering::SeqCst) {
                None
            } else {
                outcome.err().map(|err| err.to_string())
            }
        });
        self.client = Some(client);
        Ok(Some(monitor))
    }

    async fn disconnect(&mut self) {
        self.disconnecting.store(true, Ordering::SeqCst);
        self.client.take();
    }

    async fn execute(&mut self, sql: String, limit: usize) -> Result<QueryResult> {
        let client = self.client()?;
        let started = Instant::now();
        match client.query(sql.as_str(), &[]).await {
            Ok(rows) => {
                let (columns, data_rows) = convert_rows(&rows, limit);
                Ok(QueryResult {
                    columns,
                    rows: data_rows,
                    row_count: rows.len(),
                    duration: started.elapsed(),
                    truncated: rows.len() > limit,
                })
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn fetch_schemas(&mut self) -> Result<Vec<String>> {
        const SQL: &str = "
            select schema_name
            from information_schema.schemata
            where schema_name not in ('pg_catalog', 'pg_toast', 'information_schema')
            order by schema_name
        ";
        let client = self.client()?;
        let rows = client.query(SQL, &[]).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<_, String>(0).ok())
            .collect())
    }

    async fn fetch_tables(&mut self, schema: String) -> Result<Vec<String>> {
        const SQL: &str = "
            select table_name
            from information_schema.tables
            where table_schema = $1 and table_type = 'BASE TABLE'
            order by table_name
        ";
        let client = self.client()?;
        let rows = client.query(SQL, &[&schema]).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<_, String>(0).ok())
            .collect())
    }

    async fn fetch_columns(
        &mut self,
        schema: String,
        table: String,
    ) -> Result<Vec<ColumnMetadata>> {
        const SQL: &str = "
            select
                column_name,
                data_type
            from information_schema.columns
            where table_schema = $1
              and table_name = $2
            order by ordinal_position
        ";
        let client = self.client()?;
        let rows = client.query(SQL, &[&schema, &table]).await?;
        Ok(rows
            .into_iter()
            .filter_map(
                |row| match (row.try_get::<_, String>(0), row.try_get::<_, String>(1)) {
                    (Ok(name), Ok(data_type)) => Some(ColumnMetadata { name, data_type }),
                    _ => None,
                },
            )
            .collect())
    }

    async fn preview_table(
        &mut self,
        schema: String,
        table: String,
        limit: usize,
    ) -> Result<QueryResult> {
        let sql = format!(
            "select * from {} limit {}",
            qualified_table_name(&schema, &table),
            limit.min(ROW_LIMIT)
        );
        let client = self.client()?;
        let started = Instant::now();
        match client.query(sql.as_str(), &[]).await {
            Ok(rows) => {
                let (columns, data_rows) = convert_rows(&rows, limit);
                Ok(QueryResult {
                    columns,
                    rows: data_rows,
                    row_count: rows.len(),
                    duration: started.elapsed(),
                    truncated: rows.len() == limit,
                })
            }
            Err(err) => Err(err.into()),
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

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn qualified_table_name(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
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

fn classify_connection_error(err: &tokio_postgres::Error) -> ConnectionError {
    use tokio_postgres::error::SqlState;

    if let Some(db_err) = err.as_db_error() {
        let detail = err.to_string();
        match db_err.code() {
            &SqlState::INVALID_PASSWORD => {
                return ConnectionError::new("Password authentication failed.", detail);
            }
            &SqlState::INVALID_AUTHORIZATION_SPECIFICATION => {
                return ConnectionError::new("User does not exist or lacks permission.", detail);
            }
            &SqlState::INVALID_CATALOG_NAME => {
                return ConnectionError::new("Database does not exist.", detail);
            }
            _ => {}
        }
        return ConnectionError::new(db_err.message().to_string(), detail);
    }

    let detail = err.to_string();
    let lower = detail.to_lowercase();
    if lower.contains("connection refused") {
        ConnectionError::new(
            "Unable to reach the database host (connection refused).",
            detail,
        )
    } else if lower.contains("timeout") {
        ConnectionError::new("Connection timed out.", detail)
    } else {
        ConnectionError::new("Failed to connect to the database.", detail)
    }
}
