use std::collections::HashSet;
mod terminal;
mod terminal_view;
#[cfg(target_family = "wasm")]
use std::{borrow::Cow, cell::RefCell};
use terminal::{History, Language};
use terminal_view::View;

use gpui::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{InputEditorStyle, InputEvent, InputState, TextareaState};
use gpui_base::{Button, Input, InputBase, Theme, h_flex, v_flex};
use gpui_graph::{Edge, GraphData, LabelMode, NetworkGraph, NetworkGraphEvent, Node, Selection};
use hawdb::{Database, QueryOutput, Value, ValueRef};
#[cfg(target_family = "wasm")]
use {
    gpui::{ApplicationHandle, WindowOptions},
    gpui_base::{ColorTokens, SemanticThemeTokens},
    wasm_bindgen::prelude::*,
};

const MAX_QUERY_BYTES: usize = 64 * 1024;
const MAX_RESULT_ROWS: usize = 100;
const MAX_RESULT_COLUMNS: usize = 12;
const DEFAULT_QUERY: &str = "MATCH (n) RETURN n.name AS name, n.kind AS kind ORDER BY name";
const NODE_QUERY: &str = "MATCH (node) RETURN node.id AS node_id, node.name AS node_name, node.kind AS node_kind, node.description AS node_description ORDER BY node_id";
const EDGE_QUERY: &str = "MATCH (source)-[relation]->(target) RETURN source.id AS source_id, relation.kind AS relation_kind, target.id AS target_id ORDER BY source_id, target_id";

const SAMPLE_NODES: &[(&str, &str, &str, &str)] = &[
    (
        "hawdb",
        "HawDB",
        "Database",
        "Embedded context and knowledge database",
    ),
    (
        "memory",
        "Agent memory",
        "Knowledge",
        "Facts retained for future reasoning",
    ),
    (
        "entity",
        "Canonical entities",
        "Knowledge",
        "Identity-bound people, projects, and concepts",
    ),
    (
        "event",
        "Event stream",
        "Knowledge",
        "Time-ordered observations and actions",
    ),
    (
        "context",
        "Bounded context",
        "Pipeline",
        "Relevant facts hydrated within a strict budget",
    ),
    (
        "retrieval",
        "Retrieval pipeline",
        "Pipeline",
        "Search, filter, graph expansion, rerank, and hydrate",
    ),
    (
        "cypher",
        "Cypher",
        "Interface",
        "Parameterized graph query language",
    ),
    (
        "sql",
        "PostgreSQL SQL",
        "Interface",
        "Relational query surface on the same snapshot",
    ),
    (
        "agent",
        "Reasoning agent",
        "Application",
        "Consumes trusted context and records new knowledge",
    ),
    (
        "browser",
        "GPUI Web",
        "Runtime",
        "Browser canvas powered by WebGPU or WebGL2",
    ),
    (
        "rust",
        "Rust core",
        "Runtime",
        "One module containing database and graph frontend",
    ),
    (
        "snapshot",
        "Canonical snapshot",
        "Database",
        "Consistent graph and row state for every read",
    ),
    (
        "schema",
        "Isolated schema note",
        "Knowledge",
        "An isolated node proving zero-degree entities remain visible",
    ),
];

const SAMPLE_EDGES: &[(&str, &str, &str)] = &[
    ("hawdb", "memory", "STORES"),
    ("hawdb", "entity", "STORES"),
    ("hawdb", "event", "STORES"),
    ("hawdb", "cypher", "QUERIED_BY"),
    ("hawdb", "sql", "QUERIED_BY"),
    ("hawdb", "snapshot", "PUBLISHES"),
    ("retrieval", "context", "PRODUCES"),
    ("retrieval", "snapshot", "READS"),
    ("retrieval", "entity", "EXPANDS"),
    ("agent", "retrieval", "REQUESTS"),
    ("agent", "memory", "WRITES"),
    ("context", "agent", "GROUNDS"),
    ("browser", "hawdb", "HOSTS"),
    ("rust", "hawdb", "POWERS"),
    ("browser", "rust", "RUNS"),
    ("snapshot", "memory", "CONTAINS"),
    ("snapshot", "event", "CONTAINS"),
    ("cypher", "entity", "MATCHES"),
];

#[cfg(target_family = "wasm")]
thread_local! {
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

pub struct DemoApp {
    database: Database,
    query: Entity<InputState>,
    graph: Entity<NetworkGraph>,
    selection: Selection,
    status: SharedString,
    error: Option<SharedString>,
    results: Option<ResultPreview>,
    terminal_input: Entity<TextareaState>,
    history: History,
    language: Language,
    view: View,
    terminal_scroll: ScrollHandle,
    graph_source: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl DemoApp {
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Enter a Cypher query…")
                .default_value(DEFAULT_QUERY)
        });
        let mut database = Database::new();
        let terminal_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Type a statement…")
                .default_value(terminal::GRAPH_QUERY)
                .rows(3)
                .submit_on_enter(true)
        });
        let initial = seed_sample(&mut database)
            .and_then(|_| terminal::seed_sql(&mut database))
            .and_then(|_| load_graph(&mut database));
        let (data, status, error) = match initial {
            Ok(data) => {
                let status = format!(
                    "{} entities · {} relationships · HawDB WASM",
                    data.nodes.len(),
                    data.edges.len()
                )
                .into();
                (data, status, None)
            }
            Err(error) => (
                GraphData::default(),
                "Initialization failed".into(),
                Some(error.into()),
            ),
        };
        let graph = cx.new(|cx| {
            NetworkGraph::new(cx)
                .with_data(data)
                .with_label_mode(LabelMode::Always)
                .arrows(true)
                .edge_labels(true)
                .tooltips(true)
        });

        let subscriptions = vec![
            cx.subscribe_in(
                &terminal_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { shift: false, .. } = event {
                        this.run_terminal(window, cx);
                    }
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.run_query(window, cx);
                }
            }),
            cx.subscribe(&graph, |this, _, event: &NetworkGraphEvent, cx| {
                if let NetworkGraphEvent::SelectionChanged(selection) = event {
                    this.selection = selection.clone();
                    cx.notify();
                }
            }),
        ];

        let this = Self {
            database,
            query,
            graph,
            selection: Selection::None,
            status,
            error,
            results: None,
            terminal_input,
            history: History::default(),
            language: Language::Cypher,
            view: View::Terminal,
            terminal_scroll: ScrollHandle::new(),
            graph_source: "Database snapshot".into(),
            _subscriptions: subscriptions,
        };
        this.style_input(cx);
        this
    }

    fn style_input(&self, cx: &mut Context<Self>) {
        let colors = Theme::global(cx).tokens.colors;
        self.query.update(cx, |state, _| {
            state.set_editor_style(InputEditorStyle {
                foreground: colors.foreground,
                muted_foreground: colors.muted_foreground,
                selection: colors.selection,
                caret: colors.primary,
                ..InputEditorStyle::default()
            });
        });
        self.terminal_input.update(cx, |state, _| {
            state.set_editor_style(InputEditorStyle {
                foreground: colors.foreground,
                muted_foreground: colors.muted_foreground,
                selection: colors.selection,
                caret: gpui::rgb(0xf5ae47).into(),
                ..InputEditorStyle::default()
            });
        });
    }

    fn run_query(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.results = None;
        if let Err(error) = validate_query(&query) {
            self.error = Some(error.into());
            self.status = "Query rejected".into();
            cx.notify();
            return;
        }

        match self.database.query(&query) {
            Ok(output) => {
                self.results = Some(ResultPreview::from_output(&output));
                match load_graph(&mut self.database) {
                    Ok(data) => {
                        self.graph_source = "Database snapshot".into();
                        let rows = output.rows.len();
                        self.status = format!(
                            "Query OK · {rows} rows · {} entities · {} relationships",
                            data.nodes.len(),
                            data.edges.len()
                        )
                        .into();
                        self.error = None;
                        self.graph.update(cx, |graph, cx| graph.set_data(data, cx));
                    }
                    Err(error) => {
                        self.status = "Query succeeded · graph unavailable".into();
                        self.error = Some(error.into());
                        self.selection = Selection::None;
                        self.graph
                            .update(cx, |graph, cx| graph.set_data(GraphData::default(), cx));
                    }
                }
            }
            Err(error) => {
                self.status = "Query failed".into();
                self.error = Some(error.to_string().into());
            }
        }
        cx.notify();
    }

    fn reset_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut database = Database::new();
        match seed_sample(&mut database)
            .and_then(|_| terminal::seed_sql(&mut database))
            .and_then(|_| load_graph(&mut database))
        {
            Ok(data) => {
                self.database = database;
                self.query
                    .update(cx, |query, cx| query.set_value(DEFAULT_QUERY, window, cx));
                self.results = None;
                self.history = History::default();
                self.language = Language::Cypher;
                self.graph_source = "Database snapshot".into();
                self.terminal_input.update(cx, |input, cx| {
                    input.set_value(terminal::GRAPH_QUERY, window, cx)
                });
                self.selection = Selection::None;
                self.status = format!(
                    "Sample reloaded · {} entities · {} relationships",
                    data.nodes.len(),
                    data.edges.len()
                )
                .into();
                self.error = None;
                self.graph.update(cx, |graph, cx| graph.set_data(data, cx));
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        h_flex()
            .flex_shrink_0()
            .px_4()
            .py_2()
            .gap_3()
            .flex_wrap()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_base()
                    .child("HawDB Graph Demo"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .p_1()
                    .rounded_md()
                    .bg(colors.background)
                    .children([View::Terminal, View::Split, View::Graph].map(|view| {
                        Button::new(view.id())
                            .h_7()
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .text_sm()
                            .bg(if self.view == view {
                                colors.accent
                            } else {
                                colors.background
                            })
                            .text_color(if self.view == view {
                                colors.foreground
                            } else {
                                colors.muted_foreground
                            })
                            .hover(move |style| style.bg(colors.accent))
                            .child(view.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.view = view;
                                if view != View::Graph {
                                    this.terminal_input
                                        .update(cx, |input, cx| input.focus(window, cx));
                                }
                                cx.notify();
                            }))
                    })),
            )
    }

    fn render_query_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let query = self.query.clone();
        h_flex()
            .flex_shrink_0()
            .w_full()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                InputBase::new("cypher-query")
                    .flex_1()
                    .min_w_0()
                    .h_9()
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.input)
                    .bg(colors.background)
                    .text_sm()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        query.update(cx, |state, cx| state.focus(window, cx));
                    })
                    .child(Input::new(&self.query)),
            )
            .child(
                primary_button("run-query", "Run", cx)
                    .on_click(cx.listener(|this, _, window, cx| this.run_query(window, cx))),
            )
    }

    fn render_sidebar(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let graph = self.graph.read(cx);
        v_flex()
            .id("sidebar")
            .w(px(if narrow { 224. } else { 320. }))
            .flex_shrink_0()
            .h_full()
            .p_4()
            .gap_4()
            .border_l_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .overflow_y_scroll()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.muted_foreground)
                    .child("ENTITY TYPES"),
            )
            .child(
                v_flex()
                    .gap_2()
                    .children(graph.categories().into_iter().map(|category| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .child(div().size_2p5().rounded_full().bg(category.color))
                            .child(div().flex_1().child(category.name))
                            .child(
                                div()
                                    .text_color(colors.muted_foreground)
                                    .child(category.count.to_string()),
                            )
                    })),
            )
            .child(div().h_px().bg(colors.border))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.muted_foreground)
                    .child("INSPECTOR"),
            )
            .child(self.render_selection(cx))
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.background)
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child("Real gpui-graph 0.1.1")
                    .child("HawDB 0.11 · in memory")
                    .child("WebGPU / WebGL2"),
            )
    }

    fn render_results(&self, result: &ResultPreview, cx: &App) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        v_flex()
            .h(px(190.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .flex_wrap()
                    .gap_2()
                    .text_xs()
                    .child("RESULTS")
                    .child(div().text_color(colors.muted_foreground).child(format!(
                        "{}/{} rows · {}/{} columns",
                        result.rows.len(),
                        result.total_rows,
                        result.columns.len(),
                        result.total_columns
                    ))),
            )
            .child(
                div()
                    .id("query-results")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(
                        v_flex()
                            .min_w_full()
                            .child(h_flex().children(result.columns.iter().map(|column| {
                                div()
                                    .w(px(220.))
                                    .flex_shrink_0()
                                    .px_4()
                                    .py_1()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .border_b_1()
                                    .border_color(colors.border)
                                    .child(column.clone())
                            })))
                            .children(result.rows.iter().map(|row| {
                                h_flex().children(row.iter().map(|cell| {
                                    div()
                                        .w(px(220.))
                                        .flex_shrink_0()
                                        .px_4()
                                        .py_1()
                                        .text_xs()
                                        .overflow_hidden()
                                        .child(cell.clone())
                                }))
                            }))
                            .children(result.rows.is_empty().then(|| {
                                div()
                                    .px_4()
                                    .py_2()
                                    .text_sm()
                                    .text_color(colors.muted_foreground)
                                    .child("Query completed with no returned rows.")
                            })),
                    ),
            )
    }

    fn render_selection(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        let graph = self.graph.read(cx);
        match &self.selection {
            Selection::Node(node) => v_flex()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(node.category.clone()),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(node.label.clone()),
                        ),
                )
                .child(info_row("id", node.id.clone(), colors.muted_foreground))
                .children(node.properties.iter().map(|(key, value)| {
                    info_row(key.clone(), value.clone(), colors.muted_foreground)
                }))
                .child(info_row(
                    "connections",
                    graph.degree(&node.id).to_string().into(),
                    colors.muted_foreground,
                ))
                .into_any_element(),
            Selection::Edge(edge) => v_flex()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(edge.label.clone()),
                )
                .child(info_row(
                    "from",
                    edge.source.clone(),
                    colors.muted_foreground,
                ))
                .child(info_row("to", edge.target.clone(), colors.muted_foreground))
                .into_any_element(),
            Selection::Group(group) => v_flex()
                .gap_2()
                .child(div().text_lg().child(group.label.clone()))
                .child(info_row(
                    "members",
                    group.size.to_string().into(),
                    colors.muted_foreground,
                ))
                .into_any_element(),
            Selection::EdgeBundle(edges) => v_flex()
                .gap_2()
                .child("Merged relationships")
                .child(info_row(
                    "edges",
                    edges.len().to_string().into(),
                    colors.muted_foreground,
                ))
                .into_any_element(),
            _ => v_flex()
                .gap_2()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child("Select a node or relationship.")
                .child("Drag nodes · scroll to zoom")
                .child("Arrows navigate · 0 fits view")
                .into_any_element(),
        }
    }
}

impl Render for DemoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        v_flex()
            .size_full()
            // Input emits PressEnter and propagates its action. Consume it here
            // so the browser does not also submit via beforeinput/insertLineBreak.
            .on_action(|_: &gpui_base::input::Enter, _, cx| cx.stop_propagation())
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(self.render_toolbar(cx))
            .child(self.render_workspace(window, cx))
            .child(
                v_flex()
                    .min_h_8()
                    .flex_shrink_0()
                    .py_1()
                    .px_4()
                    .gap_1()
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(self.status.clone()))
                            .child(
                                Button::new("reset-sample")
                                    .h_6()
                                    .px_2()
                                    .rounded_sm()
                                    .hover(move |style| style.bg(colors.accent))
                                    .child("Reset sample")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_sample(window, cx)
                                    })),
                            ),
                    )
                    .children(
                        self.error
                            .clone()
                            .map(|error| div().text_color(colors.destructive).child(error)),
                    ),
            )
    }
}

struct ResultPreview {
    columns: Vec<SharedString>,
    rows: Vec<Vec<SharedString>>,
    total_rows: usize,
    total_columns: usize,
}

impl ResultPreview {
    fn from_output(output: &QueryOutput) -> Self {
        Self {
            columns: output
                .schema()
                .columns()
                .iter()
                .take(MAX_RESULT_COLUMNS)
                .map(|column| preview_text(column))
                .collect(),
            rows: output
                .value_rows()
                .take(MAX_RESULT_ROWS)
                .map(|row| {
                    row.iter()
                        .take(MAX_RESULT_COLUMNS)
                        .map(|value| preview_text(&value.to_string()))
                        .collect()
                })
                .collect(),
            total_rows: output.rows.len(),
            total_columns: output.schema().columns().len(),
        }
    }
}

fn preview_text(value: &str) -> SharedString {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview.into()
}

fn validate_query(query: &str) -> Result<(), &'static str> {
    if query.trim().is_empty() {
        Err("Enter a Cypher query first.")
    } else if query.len() > MAX_QUERY_BYTES {
        Err("Query exceeds the 64 KiB input limit.")
    } else {
        Ok(())
    }
}

fn primary_button(id: &'static str, label: &'static str, cx: &App) -> Button {
    let colors = Theme::global(cx).tokens.colors;
    Button::new(id)
        .h_9()
        .px_4()
        .flex()
        .items_center()
        .rounded_md()
        .bg(colors.primary)
        .text_color(colors.primary_foreground)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .hover(move |style| style.bg(colors.primary.opacity(0.85)))
        .child(label)
}

fn ghost_button(id: &'static str, label: &'static str, cx: &App) -> Button {
    let colors = Theme::global(cx).tokens.colors;
    Button::new(id)
        .h_9()
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .text_color(colors.foreground)
        .text_sm()
        .hover(move |style| style.bg(colors.accent))
        .child(label)
}

fn info_row(
    key: impl Into<SharedString>,
    value: SharedString,
    muted: gpui::Hsla,
) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(div().text_xs().text_color(muted).child(key.into()))
        .child(div().text_sm().child(value))
}

fn seed_sample(database: &mut Database) -> Result<(), String> {
    database
        .query("MATCH (n) DETACH DELETE n")
        .map_err(|error| error.to_string())?;
    for (id, name, kind, description) in SAMPLE_NODES {
        let query = format!(
            "CREATE (:Entity {{id: '{}', name: '{}', kind: '{}', description: '{}'}})",
            escape_cypher(id),
            escape_cypher(name),
            escape_cypher(kind),
            escape_cypher(description)
        );
        database.query(&query).map_err(|error| error.to_string())?;
    }
    for (source, target, kind) in SAMPLE_EDGES {
        let query = format!(
            "MATCH (a {{id: '{}'}}), (b {{id: '{}'}}) CREATE (a)-[:RELATED {{kind: '{}'}}]->(b)",
            escape_cypher(source),
            escape_cypher(target),
            escape_cypher(kind)
        );
        database.query(&query).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn escape_cypher(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn load_graph(database: &mut Database) -> Result<GraphData, String> {
    let nodes = database
        .query(NODE_QUERY)
        .map_err(|error| error.to_string())?;
    let edges = database
        .query(EDGE_QUERY)
        .map_err(|error| error.to_string())?;
    graph_data(&nodes, &edges)
}

fn graph_data(nodes: &QueryOutput, edges: &QueryOutput) -> Result<GraphData, String> {
    // HawDB may return an empty schema when a MATCH has no rows.
    if nodes.rows.is_empty() {
        return if edges.rows.is_empty() {
            Ok(GraphData::default())
        } else {
            Err("Relationships were returned without any nodes.".to_owned())
        };
    }
    let node_id = column(nodes, "node_id")?;
    let node_name = column(nodes, "node_name")?;
    let node_kind = column(nodes, "node_kind")?;
    let node_description = column(nodes, "node_description")?;
    let mut ids = HashSet::new();
    let mut graph_nodes = Vec::with_capacity(nodes.rows.len());

    for row in nodes.value_rows() {
        let id = required_identifier(row, node_id, "node.id")?;
        if !ids.insert(id.clone()) {
            return Err(format!("Duplicate graph id: {id}"));
        }
        let name = optional_text(row, node_name).unwrap_or_else(|| id.clone());
        let kind = optional_text(row, node_kind).unwrap_or_else(|| "Unknown".to_owned());
        let mut node = Node::new(id, name, kind);
        if let Some(description) = optional_text(row, node_description) {
            node = node.property("description", description);
        }
        graph_nodes.push(node);
    }

    if edges.rows.is_empty() {
        return Ok(GraphData::new(graph_nodes, Vec::new()));
    }
    let source_id = column(edges, "source_id")?;
    let relation_kind = column(edges, "relation_kind")?;
    let target_id = column(edges, "target_id")?;
    let mut graph_edges = Vec::with_capacity(edges.rows.len());
    for row in edges.value_rows() {
        let source = required_identifier(row, source_id, "source.id")?;
        let target = required_identifier(row, target_id, "target.id")?;
        if !ids.contains(&source) || !ids.contains(&target) {
            return Err(format!(
                "Relationship references an unknown endpoint: {source} → {target}"
            ));
        }
        let label = optional_text(row, relation_kind).unwrap_or_else(|| "RELATED_TO".to_owned());
        graph_edges.push(Edge::new(source, target, label));
    }
    Ok(GraphData::new(graph_nodes, graph_edges))
}

fn column(output: &QueryOutput, name: &str) -> Result<usize, String> {
    output
        .schema()
        .columns()
        .iter()
        .position(|column| column == name)
        .ok_or_else(|| format!("HawDB result is missing column {name}"))
}

fn required_identifier(row: &[Value], index: usize, field: &str) -> Result<String, String> {
    match row.get(index).map(Value::as_ref) {
        Some(ValueRef::String(value)) if !value.is_empty() => Ok(value.to_owned()),
        Some(ValueRef::Int(value)) => Ok(value.to_string()),
        Some(ValueRef::Uuid(value)) => Ok(value.to_string()),
        _ => Err(format!(
            "{field} must be a non-empty string, integer, or UUID"
        )),
    }
}

fn optional_text(row: &[Value], index: usize) -> Option<String> {
    match row.get(index)?.as_ref() {
        ValueRef::Null => None,
        ValueRef::String(value) => Some(value.to_owned()),
        ValueRef::Bool(value) => Some(value.to_string()),
        ValueRef::Int(value) => Some(value.to_string()),
        ValueRef::Float(value) => Some(value.to_string()),
        ValueRef::Uuid(value) => Some(value.to_string()),
        ValueRef::Binary(value) => Some(format!("{} bytes", value.len())),
        ValueRef::List(value) => Some(format!("{} items", value.len())),
        ValueRef::Map(value) => Some(format!("{} fields", value.len())),
    }
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen]
pub fn run() -> Result<(), JsValue> {
    if APPLICATION.with(|slot| slot.borrow().is_some()) {
        return Ok(());
    }
    gpui_platform::web_init();
    let application = gpui_platform::single_threaded_web();
    let handle = application.run_embedded(|cx: &mut App| {
        gpui_base::init(cx);
        // The wasm target receives gpui-base's Ctrl bindings even on a Mac.
        // Keep those and also support the browser host's Command shortcuts.
        use gpui::KeyBinding;
        use gpui_base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
        cx.bind_keys([
            KeyBinding::new("cmd-a", SelectAll, Some("Input")),
            KeyBinding::new("cmd-c", Copy, Some("Input")),
            KeyBinding::new("cmd-x", Cut, Some("Input")),
            KeyBinding::new("cmd-v", Paste, Some("Input")),
            KeyBinding::new("cmd-z", Undo, Some("Input")),
            KeyBinding::new("cmd-shift-z", Redo, Some("Input")),
        ]);
        gpui_graph::init(cx);
        terminal_view::init(cx);
        Theme::global_mut(cx).tokens = SemanticThemeTokens {
            colors: ColorTokens::dark(),
            ..Default::default()
        };
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/IBMPlexSans-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/JetBrainsMono-Regular.ttf").as_slice()),
            ])
            .expect("failed to load the bundled GPUI font");
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| DemoApp::new(window, cx))
        })
        .expect("failed to open the GPUI Web window");
        cx.activate(true);
    });
    APPLICATION.with(|slot| *slot.borrow_mut() = Some(handle));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hawdb_snapshot_maps_to_real_gpui_graph_and_keeps_isolates() {
        let mut database = Database::new();
        seed_sample(&mut database).expect("seed sample");
        let graph = load_graph(&mut database).expect("load graph");
        assert_eq!(graph.nodes.len(), SAMPLE_NODES.len());
        assert_eq!(graph.edges.len(), SAMPLE_EDGES.len());
        assert!(graph.nodes.iter().any(|node| node.id.as_ref() == "schema"));
        assert!(
            !graph
                .edges
                .iter()
                .any(|edge| edge.source.as_ref() == "schema" || edge.target.as_ref() == "schema")
        );
        assert!(graph.nodes.iter().any(|node| {
            node.id.as_ref() == "hawdb"
                && node.label.as_ref() == "HawDB"
                && node.category.as_ref() == "Database"
        }));
    }

    #[test]
    fn graph_adapter_rejects_duplicate_ids() {
        let mut database = Database::new();
        for _ in 0..2 {
            database
                .query("CREATE (:Entity {id: 'same'})")
                .expect("create duplicate");
        }
        let error = load_graph(&mut database).expect_err("duplicate ids must fail");
        assert!(error.contains("Duplicate graph id"));
    }

    #[test]
    fn invalid_query_does_not_poison_hawdb() {
        let mut database = Database::new();
        seed_sample(&mut database).expect("seed sample");
        assert!(database.query("not valid Cypher").is_err());
        let graph = load_graph(&mut database).expect("database recovers");
        assert_eq!(graph.nodes.len(), SAMPLE_NODES.len());
    }

    #[test]
    fn query_validation_rejects_blank_and_oversized_utf8_input() {
        assert!(validate_query(" \n\t").is_err());
        assert!(validate_query(&"界".repeat(MAX_QUERY_BYTES / 3 + 1)).is_err());
        assert!(validate_query(DEFAULT_QUERY).is_ok());
    }

    #[test]
    fn graph_refresh_tracks_create_delete_and_empty_database() {
        let mut database = Database::new();
        assert!(load_graph(&mut database).unwrap().nodes.is_empty());
        database.query("CREATE (:Entity {id: 'alone'})").unwrap();
        let isolated = load_graph(&mut database).unwrap();
        assert_eq!(isolated.nodes.len(), 1);
        assert!(isolated.edges.is_empty());
        seed_sample(&mut database).unwrap();
        database
            .query("CREATE (:Entity {id: 42, name: 'New node'})")
            .unwrap();
        let graph = load_graph(&mut database).unwrap();
        assert_eq!(graph.nodes.len(), SAMPLE_NODES.len() + 1);
        let created = graph
            .nodes
            .iter()
            .find(|node| node.id.as_ref() == "42")
            .unwrap();
        assert_eq!(created.category.as_ref(), "Unknown");
        database.query("MATCH (n) DETACH DELETE n").unwrap();
        let empty = load_graph(&mut database).unwrap();
        assert!(empty.nodes.is_empty() && empty.edges.is_empty());
    }

    #[test]
    fn missing_graph_identifier_can_be_repaired_with_sample_reset() {
        let mut database = Database::new();
        database
            .query("CREATE (:Entity {name: 'Missing id'})")
            .unwrap();
        assert!(load_graph(&mut database).unwrap_err().contains("node.id"));
        seed_sample(&mut database).unwrap();
        assert_eq!(
            load_graph(&mut database).unwrap().nodes.len(),
            SAMPLE_NODES.len()
        );
    }

    #[test]
    fn result_preview_preserves_schema_and_caps_rows_columns_and_unicode_cells() {
        let mut database = Database::new();
        for index in 0..105 {
            database
                .query(&format!("CREATE (:Entity {{id: {index}}})"))
                .unwrap();
        }
        let fields = (0..14)
            .map(|i| format!("n.id AS col{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let output = database
            .query(&format!("MATCH (n) RETURN {fields}"))
            .unwrap();
        let preview = ResultPreview::from_output(&output);
        assert_eq!(preview.total_rows, 105);
        assert_eq!(preview.total_columns, 14);
        assert_eq!(preview.rows.len(), MAX_RESULT_ROWS);
        assert_eq!(preview.columns.len(), MAX_RESULT_COLUMNS);
        assert_eq!(preview.columns[0].as_ref(), "col0");
        assert!(
            preview
                .rows
                .iter()
                .all(|row| row.len() == MAX_RESULT_COLUMNS)
        );
        let long = "界".repeat(241);
        assert_eq!(preview_text(&long).chars().count(), 241);
        assert!(preview_text(&long).ends_with('…'));
    }
}
