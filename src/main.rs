mod db;
mod profiles;
mod widgets;

use std::{fs, path::PathBuf, time::Duration};

use anyhow::Context as _;
use async_channel::{Receiver, Sender};
use db::{DbEvent, DbSessionHandle, QueryResult, ROW_LIMIT};
use directories::BaseDirs;
use gpui::{
    App, Application, Bounds, Context, Element, EventEmitter, IntoElement, KeyBinding, MouseButton,
    MouseUpEvent, Render, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb,
};
use profiles::{ConnectionProfile, ProfileId, ProfileStore};
use widgets::TextInput;

type Result<T> = anyhow::Result<T>;

fn main() {
    if let Err(err) = run() {
        eprintln!("DbMiru failed: {err:?}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    init_tracing();
    let config_dir = resolve_config_dir()?;
    let profile_store = ProfileStore::new(&config_dir);
    let (event_tx, event_rx) = async_channel::unbounded();

    Application::new().run({
        let mut receiver = Some(event_rx);
        let profile_store = profile_store.clone();
        let event_tx = event_tx.clone();
        move |cx: &mut App| {
            let bounds = Bounds::centered(None, gpui::size(px(1180.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(Default::default()),
                    ..Default::default()
                },
                move |_, cx| {
                    let rx = receiver.take().expect("event receiver already consumed");
                    cx.new(|cx| DbMiruApp::new(cx, profile_store.clone(), event_tx.clone(), rx))
                },
            )
            .unwrap();
            cx.activate(true);
        }
    });

    Ok(())
}

fn init_tracing() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::fmt().with_env_filter(filter).init();
    });
}

fn resolve_config_dir() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("Unable to determine config directory")?;
    let dir_name = if cfg!(target_os = "linux") {
        "dbmiru"
    } else {
        "DbMiru"
    };
    let dir = base_dirs.config_dir().join(dir_name);
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    Ok(dir)
}

actions!(app_actions, [RunQuery]);

struct DbMiruApp {
    profile_store: ProfileStore,
    profiles: Vec<ConnectionProfile>,
    selected_profile: Option<ProfileId>,
    profile_form: ProfileForm,
    profile_form_mode: ProfileFormMode,
    profile_notice: Option<String>,
    password_input: gpui::Entity<TextInput>,
    sql_input: gpui::Entity<TextInput>,
    connection: ConnectionState,
    query_state: QueryState,
    event_tx: Sender<DbEvent>,
    event_rx: Receiver<DbEvent>,
}

impl EventEmitter<RunQuery> for DbMiruApp {}

impl DbMiruApp {
    fn new(
        cx: &mut Context<Self>,
        profile_store: ProfileStore,
        event_tx: Sender<DbEvent>,
        event_rx: Receiver<DbEvent>,
    ) -> Self {
        let profiles = match profile_store.load() {
            Ok(list) => list,
            Err(err) => {
                tracing::error!("Failed to load profiles: {err:?}");
                Vec::new()
            }
        };

        let profile_form = ProfileForm::new(cx);
        let password_input = cx.new(|cx| TextInput::new(cx, "", "Password").with_obscured(true));
        let sql_input = cx.new(|cx| TextInput::new(cx, "", "SELECT 1;"));

        cx.bind_keys([
            KeyBinding::new("cmd-enter", RunQuery, Some("SqlEditor")),
            KeyBinding::new("ctrl-enter", RunQuery, Some("SqlEditor")),
        ]);

        let mut app = Self {
            profile_store,
            selected_profile: profiles.first().map(|p| p.id),
            profiles,
            profile_form,
            profile_form_mode: ProfileFormMode::Hidden,
            profile_notice: None,
            password_input,
            sql_input,
            connection: ConnectionState::default(),
            query_state: QueryState::default(),
            event_tx,
            event_rx,
        };
        app.sync_form_with_selection(cx);
        app
    }

    fn poll_events(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_db_event(event, cx);
        }
    }

    fn handle_db_event(&mut self, event: DbEvent, cx: &mut Context<Self>) {
        match event {
            DbEvent::Connected(handle) => {
                let profile_name = self
                    .selected_profile
                    .and_then(|id| self.profiles.iter().find(|p| p.id == id))
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Unknown profile".into());
                self.connection.status = ConnectionStatus::Connected(profile_name);
                self.connection.session = Some(handle);
                self.connection.last_error = None;
            }
            DbEvent::ConnectionFailed(message) => {
                self.connection.status = ConnectionStatus::Disconnected;
                self.connection.session = None;
                self.connection.last_error = Some(message);
            }
            DbEvent::ConnectionClosed(reason) => {
                self.connection.status = ConnectionStatus::Disconnected;
                self.connection.session = None;
                if let Some(reason) = reason {
                    self.connection.last_error = Some(reason);
                }
            }
            DbEvent::QueryFinished(result) => {
                self.query_state.status = QueryStatus::Idle;
                self.query_state.last_error = None;
                self.query_state.last_result = Some(QueryResultView::from(result));
            }
            DbEvent::QueryFailed(message) => {
                self.query_state.status = QueryStatus::Idle;
                self.query_state.last_result = None;
                self.query_state.last_error = Some(message);
            }
        }
        cx.notify();
    }

    fn sync_form_with_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.selected_profile
            && let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id)
        {
            let values = ProfileFormValues {
                name: profile.name.clone(),
                host: profile.host.clone(),
                port: profile.port.to_string(),
                database: profile.database.clone(),
                username: profile.username.clone(),
            };
            self.profile_form.set_values(&values, cx);
            return;
        }
        self.profile_form.clear(cx);
    }

    fn begin_create_profile(&mut self, cx: &mut Context<Self>) {
        self.profile_form_mode = ProfileFormMode::Creating;
        self.profile_notice = None;
        self.profile_form.clear(cx);
        cx.notify();
    }

    fn begin_edit_profile(&mut self, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.selected_profile {
            self.profile_form_mode = ProfileFormMode::Editing(profile_id);
            self.profile_notice = None;
            self.sync_form_with_selection(cx);
            cx.notify();
        }
    }

    fn cancel_profile_form(&mut self, cx: &mut Context<Self>) {
        self.profile_form_mode = ProfileFormMode::Hidden;
        self.profile_notice = None;
        self.sync_form_with_selection(cx);
        cx.notify();
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        let values = self.profile_form.values(cx);
        if values.name.trim().is_empty()
            || values.host.trim().is_empty()
            || values.database.trim().is_empty()
            || values.username.trim().is_empty()
        {
            self.profile_notice = Some("すべてのフィールドを入力してください".into());
            cx.notify();
            return;
        }
        let port: u16 = match values.port.trim().parse() {
            Ok(port) => port,
            Err(_) => {
                self.profile_notice = Some("ポート番号が不正です".into());
                cx.notify();
                return;
            }
        };
        let mut updated_profile = ConnectionProfile::new(
            values.name.trim().to_string(),
            values.host.trim().to_string(),
            port,
            values.database.trim().to_string(),
            values.username.trim().to_string(),
        );

        match self.profile_form_mode {
            ProfileFormMode::Creating => {
                let new_profile = updated_profile.clone();
                self.profiles.push(new_profile);
                self.selected_profile = Some(updated_profile.id);
            }
            ProfileFormMode::Editing(profile_id) => {
                if let Some(profile) = self.profiles.iter_mut().find(|p| p.id == profile_id) {
                    profile.name = updated_profile.name.clone();
                    profile.host = updated_profile.host.clone();
                    profile.port = updated_profile.port;
                    profile.database = updated_profile.database.clone();
                    profile.username = updated_profile.username.clone();
                    updated_profile.id = profile_id;
                }
                self.selected_profile = Some(profile_id);
            }
            ProfileFormMode::Hidden => {}
        }

        if let Err(err) = self.profile_store.save(&self.profiles) {
            self.profile_notice = Some(format!("保存に失敗しました: {err}"));
        } else {
            self.profile_notice = Some("保存しました".into());
            self.profile_form_mode = ProfileFormMode::Hidden;
        }
        self.sync_form_with_selection(cx);
        cx.notify();
    }

    fn delete_selected_profile(&mut self, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.selected_profile {
            self.profiles.retain(|p| p.id != profile_id);
            if let Err(err) = self.profile_store.save(&self.profiles) {
                self.profile_notice = Some(format!("削除に失敗しました: {err}"));
            } else {
                self.profile_notice = Some("プロファイルを削除しました".into());
                if let Some(current) = &self.connection.session
                    && matches!(self.connection.status, ConnectionStatus::Connected(_))
                {
                    current.disconnect();
                }
                self.connection.status = ConnectionStatus::Disconnected;
                self.connection.session = None;
            }
            self.selected_profile = self.profiles.first().map(|p| p.id);
            self.profile_form_mode = ProfileFormMode::Hidden;
            self.sync_form_with_selection(cx);
            cx.notify();
        }
    }

    fn select_profile(&mut self, profile_id: ProfileId, cx: &mut Context<Self>) {
        self.selected_profile = Some(profile_id);
        self.profile_form_mode = ProfileFormMode::Hidden;
        self.profile_notice = None;
        self.sync_form_with_selection(cx);
        cx.notify();
    }

    fn connect_selected(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_busy() {
            return;
        }
        let Some(profile_id) = self.selected_profile else {
            self.connection.last_error = Some("プロファイルを選択してください".into());
            cx.notify();
            return;
        };
        let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id).cloned() else {
            self.connection.last_error = Some("プロファイルが見つかりません".into());
            cx.notify();
            return;
        };
        let password = self.password_input.read(cx).text();

        self.connection.status = ConnectionStatus::Connecting(profile.name.clone());
        self.connection.last_error = None;
        db::spawn_session(profile, password, self.event_tx.clone());
        self.password_input.update(cx, |input, _| input.clear());
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.connection.session.take() {
            session.disconnect();
        }
        self.connection.status = ConnectionStatus::Disconnected;
        cx.notify();
    }

    fn execute_query(&mut self, cx: &mut Context<Self>) {
        if self.connection.session.is_none() {
            self.query_state.last_error = Some("まず接続してください".into());
            cx.notify();
            return;
        }
        if matches!(self.connection.status, ConnectionStatus::Connecting(_)) {
            self.query_state.last_error = Some("接続完了までお待ちください".into());
            cx.notify();
            return;
        }
        if self.query_state.status == QueryStatus::Running {
            return;
        }
        let sql = self.sql_input.read(cx).text();
        if sql.trim().is_empty() {
            self.query_state.last_error = Some("SQL を入力してください".into());
            cx.notify();
            return;
        }
        if let Some(session) = self.connection.session.as_ref() {
            self.query_state.status = QueryStatus::Running;
            self.query_state.last_error = None;
            self.query_state.last_result = None;
            session.execute(sql);
            cx.notify();
        }
    }
}

impl Render for DbMiruApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_events(cx);
        window.set_window_title("DbMiru");
        div()
            .flex()
            .size_full()
            .bg(rgb(0x0f172a))
            .text_color(rgb(0xf8fafc))
            .child(self.render_sidebar(cx))
            .child(self.render_workspace(cx))
    }
}

impl DbMiruApp {
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl Element {
        let selected = self.selected_profile;
        let mut profile_items = Vec::new();
        for profile in self.profiles.clone() {
            let is_selected = selected == Some(profile.id);
            let profile_id = profile.id;
            let name = profile.name.clone();
            let item = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded_md()
                .bg(if is_selected {
                    rgb(0x1e293b)
                } else {
                    rgb(0x111827)
                })
                .border_1()
                .border_color(rgb(0x1f2937))
                .cursor_pointer()
                .child(div().text_sm().text_color(rgb(0x93c5fd)).child(name))
                .child(div().text_xs().text_color(rgb(0x9ca3af)).child(format!(
                    "{}@{}:{}",
                    profile.username, profile.host, profile.port
                )))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                        this.select_profile(profile_id, cx)
                    }),
                );
            profile_items.push(item);
        }

        let form = self.render_profile_form(cx);

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(320.))
            .p_4()
            .gap_3()
            .bg(rgb(0x0b1120))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(0x93c5fd))
                            .child("接続プロファイル"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x2563eb))
                            .cursor_pointer()
                            .child("新規作成")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                    this.begin_create_profile(cx)
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .max_h(px(260.))
                    .children(profile_items),
            )
            .child(form)
            .child(self.render_profile_actions(cx))
    }

    fn render_profile_actions(&mut self, cx: &mut Context<Self>) -> impl Element {
        div()
            .flex()
            .gap_2()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x1d4ed8))
                    .text_sm()
                    .child("編集")
                    .cursor_pointer()
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                            this.begin_edit_profile(cx)
                        }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0xb91c1c))
                    .text_sm()
                    .child("削除")
                    .cursor_pointer()
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                            this.delete_selected_profile(cx)
                        }),
                    ),
            )
    }

    fn render_profile_form(&mut self, cx: &mut Context<Self>) -> impl Element {
        let form_visible = !matches!(self.profile_form_mode, ProfileFormMode::Hidden);
        let notice = self.profile_notice.clone();

        if !form_visible {
            return div();
        }

        let mut node = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x93c5fd))
                    .child("プロファイル編集"),
            )
            .child(self.profile_form.name.clone())
            .child(self.profile_form.host.clone())
            .child(self.profile_form.port.clone())
            .child(self.profile_form.database.clone())
            .child(self.profile_form.username.clone())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x22c55e))
                            .rounded_md()
                            .text_sm()
                            .child("保存")
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                    this.save_profile(cx)
                                }),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x374151))
                            .rounded_md()
                            .text_sm()
                            .child("キャンセル")
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                    this.cancel_profile_form(cx)
                                }),
                            ),
                    ),
            );

        if let Some(text) = notice {
            node = node.child(div().text_xs().text_color(rgb(0xfbbf24)).child(text));
        }

        node
    }

    fn render_workspace(&mut self, cx: &mut Context<Self>) -> impl Element {
        div()
            .flex()
            .flex_col()
            .flex_grow()
            .p_4()
            .gap_4()
            .child(self.render_connection_panel(cx))
            .child(self.render_editor_panel(cx))
            .child(self.render_results_panel())
    }

    fn render_connection_panel(&mut self, cx: &mut Context<Self>) -> impl Element {
        let status_text = self.connection.status_text();
        let error = self.connection.last_error.clone();
        let button_label = if self.connection.is_connected() {
            "切断"
        } else {
            "接続"
        };

        let mut panel = div()
            .flex()
            .flex_row()
            .gap_3()
            .p_4()
            .rounded_lg()
            .bg(rgb(0x111827))
            .border_1()
            .border_color(rgb(0x1f2937))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_grow()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x9ca3af))
                            .child("ステータス"),
                    )
                    .child(div().text_lg().child(status_text)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x9ca3af))
                            .child("パスワード"),
                    )
                    .child(self.password_input.clone()),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .bg(if self.connection.is_connected() {
                        rgb(0xef4444)
                    } else {
                        rgb(0x22c55e)
                    })
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .child(button_label)
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                            if this.connection.is_connected() {
                                this.disconnect(cx);
                            } else {
                                this.connect_selected(cx);
                            }
                        }),
                    ),
            );

        if let Some(text) = error {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_xs().text_color(rgb(0xf87171)).child(text)),
            );
        }

        panel
    }

    fn render_editor_panel(&mut self, cx: &mut Context<Self>) -> impl Element {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(rgb(0x111827))
            .border_1()
            .border_color(rgb(0x1f2937))
            .key_context("SqlEditor")
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| this.execute_query(cx)))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9ca3af))
                    .child("SQL エディタ"),
            )
            .child(
                div()
                    .border_1()
                    .border_color(rgb(0x1f2937))
                    .rounded_md()
                    .bg(rgb(0x0b1120))
                    .child(self.sql_input.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .bg(rgb(0x2563eb))
                            .rounded_md()
                            .text_sm()
                            .child("実行 (Cmd/Ctrl + Enter)")
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                    this.execute_query(cx)
                                }),
                            ),
                    )
                    .when(
                        matches!(self.query_state.status, QueryStatus::Running),
                        |node| node.child(div().text_sm().child("実行中...")),
                    ),
            );

        if let Some(text) = self.query_state.last_error.clone() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xf87171))
                    .child(format!("エラー: {text}")),
            );
        }

        panel
    }

    fn render_results_panel(&self) -> impl Element {
        let content = match &self.query_state.last_result {
            Some(result) => {
                let header = div()
                    .flex()
                    .border_b_1()
                    .border_color(rgb(0x1f2937))
                    .children(result.columns.iter().map(|col| {
                        div()
                            .flex_grow()
                            .text_sm()
                            .text_color(rgb(0x93c5fd))
                            .p_2()
                            .child(col.clone())
                    }));

                let rows = result.rows.iter().map(|row| {
                    div()
                        .flex()
                        .border_b_1()
                        .border_color(rgb(0x1f2937))
                        .children(
                            row.iter()
                                .map(|cell| div().flex_grow().p_2().text_sm().child(cell.clone())),
                        )
                });

                let meta = if result.truncated {
                    format!(
                        "{} 行 ({} ms, 上位 {} 行を表示 / 最大 {ROW_LIMIT})",
                        result.row_count,
                        result.duration.as_millis(),
                        result.rows.len()
                    )
                } else {
                    format!(
                        "{} 行 ({} ms)",
                        result.row_count,
                        result.duration.as_millis()
                    )
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(rgb(0x9ca3af)).child(meta))
                    .child(div().flex().flex_col().child(header).children(rows))
            }
            None => {
                div()
                    .text_sm()
                    .text_color(rgb(0x9ca3af))
                    .child(match self.query_state.status {
                        QueryStatus::Running => "クエリを実行しています...",
                        QueryStatus::Idle => "結果がここに表示されます",
                    })
            }
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(rgb(0x111827))
            .border_1()
            .border_color(rgb(0x1f2937))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9ca3af))
                    .child("結果 / エラー"),
            )
            .child(content)
    }
}

#[derive(Default)]
struct ConnectionState {
    status: ConnectionStatus,
    session: Option<DbSessionHandle>,
    last_error: Option<String>,
}

impl ConnectionState {
    fn is_connected(&self) -> bool {
        matches!(self.status, ConnectionStatus::Connected(_))
    }

    fn is_busy(&self) -> bool {
        matches!(self.status, ConnectionStatus::Connecting(_))
    }

    fn status_text(&self) -> String {
        match &self.status {
            ConnectionStatus::Disconnected => "切断済み".into(),
            ConnectionStatus::Connecting(name) => format!("{name} に接続中..."),
            ConnectionStatus::Connected(name) => format!("{name} に接続しました"),
        }
    }
}

#[derive(Default, PartialEq)]
enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting(String),
    Connected(String),
}

#[derive(Default)]
struct QueryState {
    status: QueryStatus,
    last_error: Option<String>,
    last_result: Option<QueryResultView>,
}

#[derive(Default, PartialEq)]
enum QueryStatus {
    #[default]
    Idle,
    Running,
}

struct QueryResultView {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    row_count: usize,
    duration: Duration,
    truncated: bool,
}

impl From<QueryResult> for QueryResultView {
    fn from(value: QueryResult) -> Self {
        Self {
            columns: value.columns,
            rows: value.rows,
            row_count: value.row_count,
            duration: value.duration,
            truncated: value.truncated,
        }
    }
}

struct ProfileForm {
    name: gpui::Entity<TextInput>,
    host: gpui::Entity<TextInput>,
    port: gpui::Entity<TextInput>,
    database: gpui::Entity<TextInput>,
    username: gpui::Entity<TextInput>,
}

impl ProfileForm {
    fn new(cx: &mut Context<DbMiruApp>) -> Self {
        Self {
            name: cx.new(|cx| TextInput::new(cx, "", "名前")),
            host: cx.new(|cx| TextInput::new(cx, "", "ホスト")),
            port: cx.new(|cx| TextInput::new(cx, "5432", "ポート")),
            database: cx.new(|cx| TextInput::new(cx, "", "データベース")),
            username: cx.new(|cx| TextInput::new(cx, "", "ユーザー名")),
        }
    }

    fn values(&self, cx: &mut Context<DbMiruApp>) -> ProfileFormValues {
        ProfileFormValues {
            name: self.name.read(cx).text(),
            host: self.host.read(cx).text(),
            port: self.port.read(cx).text(),
            database: self.database.read(cx).text(),
            username: self.username.read(cx).text(),
        }
    }

    fn set_values(&self, values: &ProfileFormValues, cx: &mut Context<DbMiruApp>) {
        self.name
            .update(cx, |input, _| input.set_text(&values.name));
        self.host
            .update(cx, |input, _| input.set_text(&values.host));
        self.port
            .update(cx, |input, _| input.set_text(&values.port));
        self.database
            .update(cx, |input, _| input.set_text(&values.database));
        self.username
            .update(cx, |input, _| input.set_text(&values.username));
    }

    fn clear(&self, cx: &mut Context<DbMiruApp>) {
        self.name.update(cx, |input, _| input.clear());
        self.host.update(cx, |input, _| input.clear());
        self.port.update(cx, |input, _| input.set_text("5432"));
        self.database.update(cx, |input, _| input.clear());
        self.username.update(cx, |input, _| input.clear());
    }
}

struct ProfileFormValues {
    name: String,
    host: String,
    port: String,
    database: String,
    username: String,
}

#[derive(Clone, Copy, Default)]
enum ProfileFormMode {
    #[default]
    Hidden,
    Creating,
    Editing(ProfileId),
}
