mod postgres;

use std::{
    future::Future,
    pin::Pin,
    sync::mpsc::{self, Sender as BlockingSender},
    thread,
};

use anyhow::Error;
use async_channel::Sender;
use dbmiru_core::Result;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub use postgres::PostgresAdapter;

pub const ROW_LIMIT: usize = 1000;
pub const PREVIEW_LIMIT: usize = 50;

pub type ConnectionClosedFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;

#[derive(Clone)]
pub struct ColumnMetadata {
    pub name: String,
    pub data_type: String,
}

pub enum DbEvent {
    Connected(DbSessionHandle),
    ConnectionFailed(ConnectionError),
    ConnectionClosed(Option<String>),
    QueryFinished(QueryResult),
    QueryFailed(String),
    SchemasLoaded(Vec<String>),
    TablesLoaded {
        schema: String,
        tables: Vec<String>,
    },
    ColumnsLoaded {
        schema: String,
        table: String,
        columns: Vec<ColumnMetadata>,
    },
    TablePreviewReady {
        schema: String,
        table: String,
        result: QueryResult,
    },
    MetadataFailed(String),
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub duration: std::time::Duration,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct ConnectionError {
    pub user_message: String,
    pub detail: String,
}

impl ConnectionError {
    pub fn new(user_message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            user_message: user_message.into(),
            detail: detail.into(),
        }
    }
}

#[async_trait::async_trait]
pub trait DbAdapter: Send {
    async fn connect(
        &mut self,
    ) -> std::result::Result<Option<ConnectionClosedFuture>, ConnectionError>;
    async fn disconnect(&mut self);
    async fn execute(&mut self, sql: String, limit: usize) -> Result<QueryResult>;
    async fn fetch_schemas(&mut self) -> Result<Vec<String>>;
    async fn fetch_tables(&mut self, schema: String) -> Result<Vec<String>>;
    async fn fetch_columns(&mut self, schema: String, table: String)
    -> Result<Vec<ColumnMetadata>>;
    async fn preview_table(
        &mut self,
        schema: String,
        table: String,
        limit: usize,
    ) -> Result<QueryResult>;
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

    pub fn load_schemas(&self) {
        let _ = self.commands.send(DbCommand::FetchSchemas);
    }

    pub fn load_tables(&self, schema: String) {
        let _ = self.commands.send(DbCommand::FetchTables { schema });
    }

    pub fn load_columns(&self, schema: String, table: String) {
        let _ = self
            .commands
            .send(DbCommand::FetchColumns { schema, table });
    }

    pub fn preview_table(&self, schema: String, table: String, limit: usize) {
        let _ = self.commands.send(DbCommand::PreviewTable {
            schema,
            table,
            limit,
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
    Execute {
        sql: String,
        limit: usize,
    },
    FetchSchemas,
    FetchTables {
        schema: String,
    },
    FetchColumns {
        schema: String,
        table: String,
    },
    PreviewTable {
        schema: String,
        table: String,
        limit: usize,
    },
    Disconnect,
}

pub fn spawn_session<A>(adapter: A, event_tx: Sender<DbEvent>)
where
    A: DbAdapter + 'static,
{
    let (ready_tx, ready_rx) = mpsc::channel::<UnboundedSender<DbCommand>>();
    let worker_event_tx = event_tx.clone();
    let handshake_event_tx = event_tx;
    let failure_tx = handshake_event_tx.clone();
    let join_handle = thread::spawn(move || {
        if let Err(err) = run_worker(Box::new(adapter), ready_tx, worker_event_tx) {
            let failure =
                ConnectionError::new("Failed to connect to database worker.", err.to_string());
            let _ = failure_tx.send_blocking(DbEvent::ConnectionFailed(failure));
        }
    });

    thread::spawn(move || match ready_rx.recv() {
        Ok(command_tx) => {
            let handle = DbSessionHandle::new(command_tx, join_handle);
            let _ = handshake_event_tx.send_blocking(DbEvent::Connected(handle));
        }
        Err(_) => {
            let _ = join_handle.join();
        }
    });
}

fn run_worker(
    mut adapter: Box<dyn DbAdapter>,
    ready_tx: BlockingSender<UnboundedSender<DbCommand>>,
    event_tx: Sender<DbEvent>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let (command_tx, mut command_rx) = unbounded_channel::<DbCommand>();

        let connection_future = match adapter.connect().await {
            Ok(connection_future) => connection_future,
            Err(error) => {
                let _ = event_tx.send(DbEvent::ConnectionFailed(error)).await;
                return Ok::<(), Error>(());
            }
        };

        if ready_tx.send(command_tx).is_err() {
            adapter.disconnect().await;
            return Ok::<(), Error>(());
        }

        if let Some(fut) = connection_future {
            spawn_connection_monitor(fut, event_tx.clone());
        }

        process_commands(adapter.as_mut(), &mut command_rx, event_tx.clone()).await;

        adapter.disconnect().await;
        Ok(())
    })?;

    Ok(())
}

fn spawn_connection_monitor(future: ConnectionClosedFuture, event_tx: Sender<DbEvent>) {
    tokio::spawn(async move {
        let reason = future.await;
        let _ = event_tx.send(DbEvent::ConnectionClosed(reason)).await;
    });
}

async fn process_commands(
    adapter: &mut dyn DbAdapter,
    command_rx: &mut UnboundedReceiver<DbCommand>,
    event_tx: Sender<DbEvent>,
) {
    while let Some(command) = command_rx.recv().await {
        match command {
            DbCommand::Execute { sql, limit } => match adapter.execute(sql, limit).await {
                Ok(result) => {
                    let _ = event_tx.send(DbEvent::QueryFinished(result)).await;
                }
                Err(err) => {
                    let _ = event_tx.send(DbEvent::QueryFailed(err.to_string())).await;
                }
            },
            DbCommand::FetchSchemas => match adapter.fetch_schemas().await {
                Ok(schemas) => {
                    let _ = event_tx.send(DbEvent::SchemasLoaded(schemas)).await;
                }
                Err(err) => {
                    let _ = event_tx
                        .send(DbEvent::MetadataFailed(format!(
                            "Failed to load schemas: {err}"
                        )))
                        .await;
                }
            },
            DbCommand::FetchTables { schema } => match adapter.fetch_tables(schema.clone()).await {
                Ok(tables) => {
                    let _ = event_tx
                        .send(DbEvent::TablesLoaded { schema, tables })
                        .await;
                }
                Err(err) => {
                    let _ = event_tx
                        .send(DbEvent::MetadataFailed(format!(
                            "Failed to load tables: {err}"
                        )))
                        .await;
                }
            },
            DbCommand::FetchColumns { schema, table } => {
                match adapter.fetch_columns(schema.clone(), table.clone()).await {
                    Ok(columns) => {
                        let _ = event_tx
                            .send(DbEvent::ColumnsLoaded {
                                schema,
                                table,
                                columns,
                            })
                            .await;
                    }
                    Err(err) => {
                        let _ = event_tx
                            .send(DbEvent::MetadataFailed(format!(
                                "Failed to load columns: {err}"
                            )))
                            .await;
                    }
                }
            }
            DbCommand::PreviewTable {
                schema,
                table,
                limit,
            } => match adapter
                .preview_table(schema.clone(), table.clone(), limit)
                .await
            {
                Ok(result) => {
                    let _ = event_tx
                        .send(DbEvent::TablePreviewReady {
                            schema,
                            table,
                            result,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = event_tx
                        .send(DbEvent::MetadataFailed(format!(
                            "Failed to preview table: {err}"
                        )))
                        .await;
                }
            },
            DbCommand::Disconnect => {
                adapter.disconnect().await;
                break;
            }
        }
    }
}
