mod db;
mod profiles;
mod widgets;

use std::{borrow::Cow, fs, path::PathBuf, time::Duration};

use anyhow::Context as _;
use async_channel::{Receiver, Sender};
use db::{ColumnMetadata, DbEvent, DbSessionHandle, PREVIEW_LIMIT, QueryResult, ROW_LIMIT};
use directories::BaseDirs;
use gpui::{
    AnyElement, App, Application, Bounds, ClipboardItem, Context, Element, EventEmitter,
    IntoElement, KeyBinding, MouseButton, MouseUpEvent, Render, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb,
};
use profiles::{ConnectionProfile, ProfileId, ProfileStore};
use widgets::TextInput;

type Result<T> = anyhow::Result<T>;
const LIST_SCROLL_MAX_HEIGHT: f32 = 190.;
const RESULT_COL_MIN_WIDTH: f32 = 160.;
const RESULT_NUMBER_WIDTH: f32 = 64.;
const APP_FONT_FAMILY: &str = "Zed Mono";
const CONNECTING_TICK_FRAMES: u8 = 18;

trait ScrollOverflowExt {
    fn overflow_scroll(self) -> Self;
    fn overflow_y_scroll(self) -> Self;
    fn restrict_scroll_to_axis(self) -> Self;
}

impl ScrollOverflowExt for gpui::Div {
    fn overflow_scroll(mut self) -> Self {
        self.style().overflow.x = Some(gpui::Overflow::Scroll);
        self.style().overflow.y = Some(gpui::Overflow::Scroll);
        self
    }

    fn overflow_y_scroll(mut self) -> Self {
        self.style().overflow.y = Some(gpui::Overflow::Scroll);
        self
    }

    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}

trait AlignSelfExt {
    fn align_self_end(self) -> Self;
}

impl AlignSelfExt for gpui::Div {
    fn align_self_end(mut self) -> Self {
        self.style().align_self = Some(gpui::AlignSelf::FlexEnd);
        self
    }
}

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
            register_zed_fonts(cx);
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

fn register_zed_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/zed-mono-regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/zed-mono-medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/zed-mono-semibold.ttf")),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("Failed to register bundled fonts: {err:?}");
    }
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
    schema_browser: SchemaBrowserState,
    active_tab: MainTab,
    event_tx: Sender<DbEvent>,
    event_rx: Receiver<DbEvent>,
    connecting_indicator: u8,
    connecting_indicator_frame: u8,
    connecting_indicator_active: bool,
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
            schema_browser: SchemaBrowserState::default(),
            active_tab: MainTab::default(),
            event_tx,
            event_rx,
            connecting_indicator: 0,
            connecting_indicator_frame: 0,
            connecting_indicator_active: false,
        };
        app.sync_form_with_selection(cx);
        app
    }

    fn ensure_connecting_indicator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting_indicator_active {
            return;
        }
        self.connecting_indicator_active = true;
        self.schedule_connecting_indicator(window, cx);
    }

    fn schedule_connecting_indicator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connecting_indicator_active {
            return;
        }
        cx.on_next_frame(window, |this, window, cx| {
            if !this.connection.is_busy() {
                this.stop_connecting_indicator();
                cx.notify();
                return;
            }
            this.connecting_indicator_frame = this.connecting_indicator_frame.wrapping_add(1);
            if this.connecting_indicator_frame % CONNECTING_TICK_FRAMES == 0 {
                this.connecting_indicator = (this.connecting_indicator + 1) % 4;
                cx.notify();
            }
            this.schedule_connecting_indicator(window, cx);
        });
    }

    fn stop_connecting_indicator(&mut self) {
        self.connecting_indicator_active = false;
        self.connecting_indicator = 0;
        self.connecting_indicator_frame = 0;
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
                self.stop_connecting_indicator();
                self.schema_browser.start_schema_load();
                self.active_tab = MainTab::SchemaBrowser;
                if let Some(session) = self.connection.session.as_ref() {
                    session.load_schemas();
                }
            }
            DbEvent::ConnectionFailed(message) => {
                self.connection.status = ConnectionStatus::Disconnected;
                self.connection.session = None;
                self.connection.last_error = Some(message);
                self.stop_connecting_indicator();
                self.schema_browser.reset();
                self.active_tab = MainTab::SchemaBrowser;
            }
            DbEvent::ConnectionClosed(reason) => {
                self.connection.status = ConnectionStatus::Disconnected;
                self.connection.session = None;
                if let Some(reason) = reason {
                    self.connection.last_error = Some(reason);
                }
                self.stop_connecting_indicator();
                self.schema_browser.reset();
                self.active_tab = MainTab::SchemaBrowser;
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
            DbEvent::SchemasLoaded(schemas) => {
                self.schema_browser.schemas_loading = false;
                self.schema_browser.schemas = schemas;
                self.schema_browser.last_error = None;
                if self.schema_browser.schemas.is_empty() {
                    self.schema_browser.selected_schema = None;
                } else if self.schema_browser.selected_schema.is_none() {
                    if let Some(first) = self.schema_browser.schemas.first().cloned() {
                        self.select_schema(first, cx);
                    }
                }
            }
            DbEvent::TablesLoaded { schema, tables } => {
                if self.schema_browser.selected_schema.as_deref() == Some(schema.as_str()) {
                    self.schema_browser.tables_loading = false;
                    self.schema_browser.tables = tables;
                    self.schema_browser.last_error = None;
                    if self.schema_browser.tables.is_empty() {
                        self.schema_browser.selected_table = None;
                        self.schema_browser.columns.clear();
                        self.schema_browser.preview = None;
                    } else if self.schema_browser.selected_table.is_none() {
                        if let Some(first) = self.schema_browser.tables.first().cloned() {
                            self.select_table(first, cx);
                        }
                    }
                }
            }
            DbEvent::ColumnsLoaded {
                schema,
                table,
                columns,
            } => {
                if self.schema_browser.selected_schema.as_deref() == Some(schema.as_str())
                    && self.schema_browser.selected_table.as_deref() == Some(table.as_str())
                {
                    self.schema_browser.columns_loading = false;
                    self.schema_browser.columns = columns;
                    self.schema_browser.last_error = None;
                }
            }
            DbEvent::TablePreviewReady {
                schema,
                table,
                result,
            } => {
                if self.schema_browser.selected_schema.as_deref() == Some(schema.as_str())
                    && self.schema_browser.selected_table.as_deref() == Some(table.as_str())
                {
                    self.schema_browser.preview_loading = false;
                    self.schema_browser.preview = Some(QueryResultView::from(result));
                    self.schema_browser.last_error = None;
                }
            }
            DbEvent::MetadataFailed(message) => {
                self.schema_browser.last_error = Some(message);
                self.schema_browser.stop_loading();
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
            self.profile_notice = Some("Please fill out every field.".into());
            cx.notify();
            return;
        }
        let port: u16 = match values.port.trim().parse() {
            Ok(port) => port,
            Err(_) => {
                self.profile_notice = Some("Invalid port number.".into());
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
            false,
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
            self.profile_notice = Some(format!("Failed to save: {err}"));
        } else {
            self.profile_notice = Some("Saved.".into());
            self.profile_form_mode = ProfileFormMode::Hidden;
        }
        self.sync_form_with_selection(cx);
        cx.notify();
    }

    fn delete_selected_profile(&mut self, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.selected_profile {
            self.profiles.retain(|p| p.id != profile_id);
            if let Err(err) = self.profile_store.save(&self.profiles) {
                self.profile_notice = Some(format!("Failed to delete: {err}"));
            } else {
                self.profile_notice = Some("Profile deleted.".into());
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
            self.connection.last_error = Some("Select a profile first.".into());
            cx.notify();
            return;
        };
        let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id).cloned() else {
            self.connection.last_error = Some("Profile not found.".into());
            cx.notify();
            return;
        };
        let password = self.password_input.read(cx).text();

        self.connection.status = ConnectionStatus::Connecting(profile.name.clone());
        self.connection.last_error = None;
        self.connecting_indicator = 1;
        self.connecting_indicator_frame = 0;
        self.connecting_indicator_active = false;
        db::spawn_session(profile, password, self.event_tx.clone());
        self.password_input.update(cx, |input, _| input.clear());
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.connection.session.take() {
            session.disconnect();
        }
        self.connection.status = ConnectionStatus::Disconnected;
        self.schema_browser.reset();
        self.active_tab = MainTab::SchemaBrowser;
        self.stop_connecting_indicator();
        cx.notify();
    }

    fn execute_query(&mut self, cx: &mut Context<Self>) {
        if self.connection.session.is_none() {
            self.query_state.last_error = Some("Connect to a database first.".into());
            cx.notify();
            return;
        }
        if matches!(self.connection.status, ConnectionStatus::Connecting(_)) {
            self.query_state.last_error = Some("Please wait for the connection to finish.".into());
            cx.notify();
            return;
        }
        if self.query_state.status == QueryStatus::Running {
            return;
        }
        let sql = self.sql_input.read(cx).text();
        if sql.trim().is_empty() {
            self.query_state.last_error = Some("Enter a SQL statement.".into());
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

    fn copy_to_clipboard(&mut self, value: String, cx: &mut Context<Self>) {
        let _ = cx.write_to_clipboard(ClipboardItem::new_string(value));
    }

    fn select_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        let Some(session) = self.connection.session.as_ref() else {
            self.schema_browser.last_error =
                Some("Load schemas after establishing a connection.".into());
            cx.notify();
            return;
        };
        self.schema_browser.selected_schema = Some(schema.clone());
        self.schema_browser.selected_table = None;
        self.schema_browser.tables.clear();
        self.schema_browser.columns.clear();
        self.schema_browser.preview = None;
        self.schema_browser.tables_loading = true;
        self.schema_browser.columns_loading = false;
        self.schema_browser.preview_loading = false;
        session.load_tables(schema);
        cx.notify();
    }

    fn select_table(&mut self, table: String, cx: &mut Context<Self>) {
        let Some(schema) = self.schema_browser.selected_schema.clone() else {
            return;
        };
        let Some(session) = self.connection.session.as_ref() else {
            return;
        };
        self.schema_browser.selected_table = Some(table.clone());
        self.schema_browser.columns.clear();
        self.schema_browser.preview = None;
        self.schema_browser.columns_loading = true;
        self.schema_browser.preview_loading = true;
        session.load_columns(schema.clone(), table.clone());
        session.preview_table(schema, table, db::PREVIEW_LIMIT);
        cx.notify();
    }
}

impl Render for DbMiruApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_events(cx);
        window.set_window_title("DbMiru");
        if self.connection.is_busy() {
            self.ensure_connecting_indicator(window, cx);
        } else if self.connecting_indicator_active {
            self.stop_connecting_indicator();
        }
        div()
            .flex()
            .font_family(APP_FONT_FAMILY)
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
                            .child("Connection Profiles"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x2563eb))
                            .cursor_pointer()
                            .child("New")
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
                    .child("Edit")
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
                    .child("Delete")
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
                    .child("Profile Details"),
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
                            .child("Save")
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
                            .child("Cancel")
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
            .min_w(px(0.))
            .h_full()
            .overflow_y_scroll()
            .id("workspace_scroll")
            .p_4()
            .gap_4()
            .child(self.render_connection_panel(cx))
            .child(self.render_main_tabs(cx))
    }

    fn render_connection_panel(&mut self, cx: &mut Context<Self>) -> impl Element {
        let dot_count = if self.connection.is_busy() {
            self.connecting_indicator as usize
        } else {
            0
        };
        let status_text = self.connection.status_text(dot_count);
        let error = self.connection.last_error.clone();
        let button_label = if self.connection.is_connected() {
            "Disconnect"
        } else {
            "Connect"
        };

        let mut panel = div()
            .flex()
            .flex_row()
            .items_center()
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
                    .child(div().text_sm().text_color(rgb(0x9ca3af)).child("Status"))
                    .child(div().text_lg().child(status_text)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w(px(220.))
                    .child(div().text_sm().text_color(rgb(0x9ca3af)).child("Password"))
                    .child(self.password_input.clone()),
            )
            .child(
                div()
                    .align_self_end()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .h(px(36.))
                    .px_4()
                    .rounded_lg()
                    .text_sm()
                    .text_color(rgb(0xf8fafc))
                    .bg(if self.connection.is_connected() {
                        rgb(0xef4444)
                    } else {
                        rgb(0x22c55e)
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(connection_action_icon(&self.connection.status))
                            .child(button_label),
                    )
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

    fn render_main_tabs(&mut self, cx: &mut Context<Self>) -> impl Element {
        let tabs = [
            (MainTab::SchemaBrowser, "Schema Browser"),
            (MainTab::SqlEditor, "SQL Editor"),
        ];
        let mut tab_buttons = Vec::new();
        for (tab, label) in tabs {
            let is_active = self.active_tab == tab;
            let tab_value = tab;
            tab_buttons.push(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_sm()
                    .bg(if is_active {
                        rgb(0x2563eb)
                    } else {
                        rgb(0x1f2937)
                    })
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.active_tab = tab_value;
                            cx.notify();
                        }),
                    ),
            );
        }

        let content: AnyElement = match self.active_tab {
            MainTab::SchemaBrowser => self.render_schema_browser(cx).into_any(),
            MainTab::SqlEditor => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.render_editor_panel(cx))
                .child(self.render_results_panel())
                .into_any(),
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().gap_2().children(tab_buttons))
            .child(content)
    }

    fn render_schema_browser(&mut self, cx: &mut Context<Self>) -> impl Element {
        let schema_list: AnyElement = if self.schema_browser.schemas_loading {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Loading schemas...")
                .into_any()
        } else if self.schema_browser.schemas.is_empty() {
            let message = if self.connection.is_connected() {
                "No schemas available."
            } else {
                "Connect to load schemas."
            };
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child(message)
                .into_any()
        } else {
            let items = self.schema_browser.schemas.iter().map(|schema| {
                let schema_name = schema.clone();
                let schema_name_for_copy = schema_name.clone();
                let is_selected = self
                    .schema_browser
                    .selected_schema
                    .as_ref()
                    .map(|current| current == schema)
                    .unwrap_or(false);
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .p_2()
                    .rounded_md()
                    .bg(if is_selected {
                        rgb(0x1e293b)
                    } else {
                        rgb(0x0b1120)
                    })
                    .border_1()
                    .border_color(rgb(0x1f2937))
                    .hover(|style| style.bg(rgb(0x1f2435)))
                    .cursor_pointer()
                    .child(div().text_sm().child(schema.clone()))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.select_schema(schema_name.clone(), cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.copy_to_clipboard(schema_name_for_copy.clone(), cx);
                        }),
                    )
            });
            div()
                .max_h(px(LIST_SCROLL_MAX_HEIGHT))
                .min_w(px(0.))
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .id("schema_list_scroll")
                .child(div().flex().flex_col().gap_1().children(items))
                .into_any()
        };

        let table_list: AnyElement = if self.schema_browser.tables_loading {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Loading tables...")
                .into_any()
        } else if self.schema_browser.selected_schema.is_none() {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Select a schema")
                .into_any()
        } else if self.schema_browser.tables.is_empty() {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("No tables found")
                .into_any()
        } else {
            let items = self.schema_browser.tables.iter().map(|table| {
                let table_name = table.clone();
                let table_name_for_copy = table_name.clone();
                let is_selected = self
                    .schema_browser
                    .selected_table
                    .as_ref()
                    .map(|current| current == table)
                    .unwrap_or(false);
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .p_2()
                    .rounded_md()
                    .bg(if is_selected {
                        rgb(0x1e293b)
                    } else {
                        rgb(0x0b1120)
                    })
                    .border_1()
                    .border_color(rgb(0x1f2937))
                    .hover(|style| style.bg(rgb(0x1f2435)))
                    .cursor_pointer()
                    .child(div().text_sm().child(table.clone()))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.select_table(table_name.clone(), cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.copy_to_clipboard(table_name_for_copy.clone(), cx);
                        }),
                    )
            });
            div()
                .max_h(px(LIST_SCROLL_MAX_HEIGHT))
                .min_w(px(0.))
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .id("table_list_scroll")
                .child(div().flex().flex_col().gap_1().children(items))
                .into_any()
        };

        let column_list: AnyElement = if self.schema_browser.columns_loading {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Loading columns...")
                .into_any()
        } else if self.schema_browser.selected_table.is_none() {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Select a table")
                .into_any()
        } else if self.schema_browser.columns.is_empty() {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("No columns found")
                .into_any()
        } else {
            let items = self.schema_browser.columns.iter().map(|column| {
                let column_name = column.name.clone();
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(0x0b1120))
                    .border_1()
                    .border_color(rgb(0x1f2937))
                    .hover(|style| style.bg(rgb(0x1f2435)))
                    .cursor_pointer()
                    .child(div().text_sm().child(column.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x93c5fd))
                            .child(column.data_type.clone()),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _window, cx| {
                            this.copy_to_clipboard(column_name.clone(), cx);
                        }),
                    )
            });
            div()
                .max_h(px(LIST_SCROLL_MAX_HEIGHT))
                .min_w(px(0.))
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .id("column_list_scroll")
                .child(div().flex().flex_col().gap_1().children(items))
                .into_any()
        };

        let mut panel =
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
                        .child("Schema Browser"),
                )
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_xs().text_color(rgb(0x93c5fd)).child("Schemas"))
                                .child(schema_list),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_xs().text_color(rgb(0x93c5fd)).child("Tables"))
                                .child(table_list),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .flex_grow()
                                .child(div().text_xs().text_color(rgb(0x93c5fd)).child("Columns"))
                                .child(column_list),
                        ),
                )
                .child(div().text_xs().text_color(rgb(0x6b7280)).child(
                    "Right-click to copy schema/table names. Left-click copies column names.",
                ))
                .child(self.render_preview_panel());

        if let Some(error) = self.schema_browser.last_error.clone() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(0xf87171))
                    .child(format!("Metadata fetch error: {error}")),
            );
        }

        panel
    }

    fn render_preview_panel(&mut self) -> impl Element {
        let header = if let (Some(schema), Some(table)) = (
            self.schema_browser.selected_schema.as_ref(),
            self.schema_browser.selected_table.as_ref(),
        ) {
            format!("Preview: {schema}.{table} (up to {PREVIEW_LIMIT} rows)")
        } else {
            "Table preview".into()
        };

        let content: AnyElement = if self.schema_browser.preview_loading {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Loading preview...")
                .into_any()
        } else if let Some(view) = self.schema_browser.preview.as_ref() {
            div()
                .max_h(px(260.))
                .w_full()
                .min_w(px(0.))
                .overflow_scroll()
                .restrict_scroll_to_axis()
                .id("preview_table_scroll")
                .child(self.render_result_table(view))
                .into_any()
        } else {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child("Select a table to see its preview")
                .into_any()
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().text_color(rgb(0x9ca3af)).child(header))
            .child(content)
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
                    .child("SQL Editor"),
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
                            .child("Run (Cmd/Ctrl + Enter)")
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
                        |node| node.child(div().text_sm().child("Running...")),
                    ),
            );

        if let Some(text) = self.query_state.last_error.clone() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xf87171))
                    .child(format!("Error: {text}")),
            );
        }

        panel
    }

    fn render_results_panel(&self) -> impl Element {
        let content = match &self.query_state.last_result {
            Some(result) => {
                let meta = if result.truncated {
                    format!(
                        "{} rows ({} ms, showing top {} / max {ROW_LIMIT})",
                        result.row_count,
                        result.duration.as_millis(),
                        result.rows.len()
                    )
                } else {
                    format!(
                        "{} rows ({} ms)",
                        result.row_count,
                        result.duration.as_millis()
                    )
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(rgb(0x9ca3af)).child(meta))
                    .child(
                        div()
                            .max_h(px(320.))
                            .w_full()
                            .min_w(px(0.))
                            .overflow_scroll()
                            .restrict_scroll_to_axis()
                            .id("result_table_scroll")
                            .child(self.render_result_table(result)),
                    )
            }
            None => {
                div()
                    .text_sm()
                    .text_color(rgb(0x9ca3af))
                    .child(match self.query_state.status {
                        QueryStatus::Running => "Query is running...",
                        QueryStatus::Idle => "Results will appear here.",
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
                    .child("Results / Errors"),
            )
            .child(content)
    }

    fn render_result_table(&self, view: &QueryResultView) -> AnyElement {
        let col_width = px(RESULT_COL_MIN_WIDTH);
        let total_width =
            px(RESULT_NUMBER_WIDTH + view.columns.len() as f32 * RESULT_COL_MIN_WIDTH);
        let header = div()
            .flex()
            .flex_shrink_0()
            .min_w(total_width)
            .border_b_1()
            .border_color(rgb(0x1f2937))
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(RESULT_NUMBER_WIDTH))
                    .text_xs()
                    .text_color(rgb(0x93c5fd))
                    .p_2()
                    .child("#"),
            )
            .children(view.columns.iter().map(|col| {
                div()
                    .flex_shrink_0()
                    .w(col_width)
                    .text_sm()
                    .text_color(rgb(0x93c5fd))
                    .p_2()
                    .child(col.clone())
            }));

        let rows = view.rows.iter().enumerate().map(|(idx, row)| {
            div()
                .flex()
                .flex_shrink_0()
                .min_w(total_width)
                .border_b_1()
                .border_color(rgb(0x1f2937))
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(RESULT_NUMBER_WIDTH))
                        .text_xs()
                        .text_color(rgb(0x93c5fd))
                        .p_2()
                        .child(format!("#{}", idx + 1)),
                )
                .children(row.iter().map(|cell| {
                    div()
                        .flex_shrink_0()
                        .w(col_width)
                        .p_2()
                        .text_sm()
                        .child(cell.clone())
                }))
        });

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w(total_width)
            .child(header)
            .children(rows)
            .into_any()
    }
}

fn connection_action_icon(status: &ConnectionStatus) -> gpui::Div {
    let (color, size) = match status {
        ConnectionStatus::Connected(_) => (rgb(0x22c55e), px(10.)),
        ConnectionStatus::Connecting(_) => (rgb(0xfbbf24), px(10.)),
        ConnectionStatus::Disconnected => (rgb(0xf87171), px(8.)),
    };

    div().w(size).h(size).rounded_full().bg(color)
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

    fn status_text(&self, dots: usize) -> String {
        match &self.status {
            ConnectionStatus::Disconnected => "Disconnected".into(),
            ConnectionStatus::Connecting(name) => {
                const DOTS: [&str; 4] = ["", ".", "..", "..."];
                let suffix = DOTS[dots.min(3)];
                format!("Connecting to {name}{suffix}")
            }
            ConnectionStatus::Connected(name) => format!("Connected to {name}"),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainTab {
    SchemaBrowser,
    SqlEditor,
}

impl Default for MainTab {
    fn default() -> Self {
        MainTab::SchemaBrowser
    }
}

struct SchemaBrowserState {
    schemas: Vec<String>,
    schemas_loading: bool,
    selected_schema: Option<String>,
    tables: Vec<String>,
    tables_loading: bool,
    selected_table: Option<String>,
    columns: Vec<ColumnMetadata>,
    columns_loading: bool,
    preview: Option<QueryResultView>,
    preview_loading: bool,
    last_error: Option<String>,
}

impl Default for SchemaBrowserState {
    fn default() -> Self {
        Self {
            schemas: Vec::new(),
            schemas_loading: false,
            selected_schema: None,
            tables: Vec::new(),
            tables_loading: false,
            selected_table: None,
            columns: Vec::new(),
            columns_loading: false,
            preview: None,
            preview_loading: false,
            last_error: None,
        }
    }
}

impl SchemaBrowserState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn start_schema_load(&mut self) {
        self.schemas_loading = true;
        self.tables_loading = false;
        self.columns_loading = false;
        self.preview_loading = false;
        self.schemas.clear();
        self.tables.clear();
        self.columns.clear();
        self.preview = None;
        self.selected_schema = None;
        self.selected_table = None;
        self.last_error = None;
    }

    fn stop_loading(&mut self) {
        self.schemas_loading = false;
        self.tables_loading = false;
        self.columns_loading = false;
        self.preview_loading = false;
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
            name: cx.new(|cx| TextInput::new(cx, "", "Name")),
            host: cx.new(|cx| TextInput::new(cx, "", "Host")),
            port: cx.new(|cx| TextInput::new(cx, "5432", "Port")),
            database: cx.new(|cx| TextInput::new(cx, "", "Database")),
            username: cx.new(|cx| TextInput::new(cx, "", "Username")),
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
