use super::*;
use gpui::{KeyBinding, rgb};
use gpui_base::input::Textarea;

gpui::actions!(terminal, [PreviousCommand, NextCommand]);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Terminal,
    Split,
    Graph,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Split => "Split",
            Self::Graph => "Graph",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Terminal => "view-terminal",
            Self::Split => "view-split",
            Self::Graph => "view-graph",
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", PreviousCommand, Some("TerminalSingleLine > Input")),
        KeyBinding::new("down", NextCommand, Some("TerminalSingleLine > Input")),
        KeyBinding::new("alt-up", PreviousCommand, Some("TerminalInput > Input")),
        KeyBinding::new("alt-down", NextCommand, Some("TerminalInput > Input")),
    ]);
}

impl DemoApp {
    pub(super) fn run_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.terminal_input.read(cx).value().to_string();
        if query.trim().is_empty() {
            return;
        }
        let entry = terminal::execute(&mut self.database, &query, self.language);
        self.status = match &entry.error {
            Some(_) => "Query failed · see terminal output".into(),
            None => format!(
                "{} · {} rows · saved in terminal history",
                entry.language.label(),
                entry.result.as_ref().map_or(0, |result| result.total_rows)
            )
            .into(),
        };
        self.error = None;
        // A scalar/table query deliberately leaves the last plotted snapshot alone.
        if self.view == View::Split {
            if let Some(graph) = &entry.graph {
                self.set_result_graph(graph.clone(), &query, cx);
            }
        }
        self.history.push(entry);
        self.terminal_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.terminal_scroll.scroll_to_bottom();
        cx.notify();
    }

    fn set_result_graph(&mut self, graph: GraphData, query: &str, cx: &mut Context<Self>) {
        self.graph_source = format!("Query result · {}", preview_text(query)).into();
        self.selection = Selection::None;
        self.graph.update(cx, |view, cx| view.set_data(graph, cx));
    }

    fn draw_history(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.history.entries.get(index) {
            if let Some(graph) = entry.graph.clone() {
                let query = entry.query.to_string();
                self.set_result_graph(graph, &query, cx);
                self.view = View::Split;
                self.terminal_scroll.scroll_to_top_of_item(index + 1);
                cx.notify();
            }
        }
    }

    fn recall(&mut self, previous: bool, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.terminal_input.read(cx).value().to_string();
        let recalled = if previous {
            self.history.previous(&draft, self.language)
        } else {
            self.history.next()
        };
        if let Some((query, language)) = recalled {
            self.language = language;
            self.terminal_input
                .update(cx, |input, cx| input.set_value(query, window, cx));
            cx.notify();
        }
    }

    fn example(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.language = language;
        self.terminal_input.update(cx, |input, cx| {
            input.set_value(
                if language == Language::Cypher {
                    terminal::GRAPH_QUERY
                } else {
                    terminal::SQL_QUERY
                },
                window,
                cx,
            );
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let amber = rgb(0xf5ae47);
        let input = self.terminal_input.clone();
        let multiline = self.terminal_input.read(cx).value().contains('\n');
        v_flex().size_full().min_h_0().min_w_0().bg(rgb(0x11151b)).font_family("JetBrains Mono")
            .child(h_flex().flex_shrink_0().px_4().py_2().gap_2().flex_wrap().items_center()
                .border_b_1().border_color(colors.border)
                .child(div().flex_1().text_sm().text_color(colors.muted_foreground).child("hawdb — cypher & sql"))
                .child(ghost_button("terminal-language", self.language.label(), cx).on_click(cx.listener(|this, _, window, cx| {
                    this.language = if this.language == Language::Cypher { Language::Sql } else { Language::Cypher };
                    this.terminal_input.update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                })))
                .child(ghost_button("graph-example", "Graph example", cx).on_click(cx.listener(|this, _, window, cx| this.example(Language::Cypher, window, cx))))
                .child(ghost_button("sql-example", "SQL example", cx).on_click(cx.listener(|this, _, window, cx| this.example(Language::Sql, window, cx))))
                .child(ghost_button("clear-terminal", "Clear", cx).on_click(cx.listener(|this, _, _, cx| {
                    this.history = History::default();
                    this.status = "Terminal cleared · database unchanged".into();
                    cx.notify();
                }))))
            .child(v_flex().id("terminal-transcript").flex_1().min_h_0().overflow_y_scroll()
                .track_scroll(&self.terminal_scroll).px_4().py_3().gap_4()
                .child(div().text_sm().text_color(colors.muted_foreground)
                    .child("Ready — graph + sightings SQL table loaded. Run a statement, then Draw graph on a graph result."))
                .children(self.history.entries.iter().enumerate().map(|(index, entry)| {
                    v_flex().gap_2().flex_shrink_0()
                        .child(h_flex().gap_2().items_start().text_sm().line_height(px(20.))
                            .child(div().w(px(60.)).flex_shrink_0().text_color(amber).child("hawdb>"))
                            .child(div().flex_1().min_w_0().text_sm().child(entry.query.clone()))
                            .child(div().text_xs().text_color(colors.muted_foreground).child(entry.language.label())))
                        .children(entry.error.as_ref().map(|error| div().pl_4().text_sm().text_color(colors.destructive).child(error.clone())))
                        .children(entry.result.as_ref().map(|result| self.terminal_table(index, result, cx)))
                        .children(entry.result.as_ref().map(|_| h_flex().flex_wrap().gap_2().items_center()
                            .child(div().text_xs().text_color(colors.muted_foreground).child(entry.graph_note.clone()))
                            .children(entry.graph.as_ref().map(|_| Button::new(("draw-result", index)).px_3().py_1().rounded_md()
                                .bg(rgb(0x2b2315)).text_color(amber).text_xs().child("Draw graph →")
                                .on_click(cx.listener(move |this, _, _, cx| this.draw_history(index, cx)))))))
                })))
            .child(h_flex().flex_shrink_0().border_t_1().border_color(colors.border).px_4().py_3().gap_2().items_start()
                .child(div().w(px(60.)).flex_shrink_0().pt_2().text_sm().line_height(px(20.)).text_color(amber).child("hawdb>"))
                .child(InputBase::new("terminal-query").flex_1().min_w_0().h(px(76.)).pt_2().text_sm().line_height(px(20.))
                    .key_context(if multiline { "TerminalInput" } else { "TerminalInput TerminalSingleLine" })
                    .on_action(cx.listener(|this, _: &PreviousCommand, window, cx| this.recall(true, window, cx)))
                    .on_action(cx.listener(|this, _: &NextCommand, window, cx| this.recall(false, window, cx)))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| input.update(cx, |input, cx| input.focus(window, cx)))
                    .child(Textarea::new(&self.terminal_input)))
                .child(primary_button("terminal-run", "Run", cx).on_click(cx.listener(|this, _, window, cx| this.run_terminal(window, cx)))))
            .child(div().flex_shrink_0().px_4().py_2().border_t_1().border_color(colors.border).text_xs().text_color(colors.muted_foreground)
                .child("Enter run · Shift+Enter newline · ↑ ↓ history · Alt+↑/↓ multiline history · last 40 queries"))
    }

    fn terminal_table(&self, index: usize, result: &ResultPreview, cx: &App) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let width = if result.columns.len() == 1 {
            640.
        } else {
            220.
        };
        v_flex()
            .gap_1()
            .pl_4()
            .child(
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(format!(
                        "{} rows · preview {}/{} columns · up to 100 rows, 240 characters/cell",
                        result.total_rows,
                        result.columns.len(),
                        result.total_columns
                    )),
            )
            .child(
                div()
                    .id(("terminal-table", index))
                    .max_h(px(240.))
                    .overflow_scroll()
                    .child(
                        v_flex()
                            .child(h_flex().children(result.columns.iter().map(|column| {
                                div()
                                    .w(px(width))
                                    .flex_shrink_0()
                                    .py_1()
                                    .pr_3()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .border_b_1()
                                    .border_color(colors.border)
                                    .child(column.clone())
                            })))
                            .children(result.rows.iter().map(|row| {
                                h_flex().children(row.iter().map(|cell| {
                                    div()
                                        .w(px(width))
                                        .flex_shrink_0()
                                        .py_1()
                                        .pr_3()
                                        .text_xs()
                                        .border_b_1()
                                        .border_color(colors.border.opacity(0.4))
                                        .child(cell.clone())
                                }))
                            }))
                            .children(
                                result
                                    .rows
                                    .is_empty()
                                    .then(|| div().text_xs().py_2().child("OK · no returned rows")),
                            ),
                    ),
            )
    }

    pub(super) fn render_workspace(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match self.view {
            View::Terminal => div()
                .flex_1()
                .min_h_0()
                .child(self.render_terminal(cx))
                .into_any_element(),
            View::Graph => h_flex()
                .flex_1()
                .min_h_0()
                // h_flex centers children by default, collapsing the graph's height.
                .items_stretch()
                .child(self.render_graph_panel(true, cx))
                .children(
                    (window.viewport_size().width >= px(560.))
                        .then(|| self.render_sidebar(window.viewport_size().width < px(900.), cx)),
                )
                .into_any_element(),
            View::Split => {
                if window.viewport_size().width >= px(1100.) {
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .items_stretch()
                        .child(div().w_1_2().h_full().child(self.render_terminal(cx)))
                        .child(self.render_graph_panel(false, cx))
                        .into_any_element()
                } else {
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .child(div().h_1_2().min_h_0().child(self.render_terminal(cx)))
                        .child(self.render_graph_panel(false, cx))
                        .into_any_element()
                }
            }
        }
    }

    fn render_graph_panel(&self, show_results: bool, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex_shrink_0()
                    .px_4()
                    .py_1()
                    .text_xs()
                    .text_color(Theme::global(cx).tokens.colors.muted_foreground)
                    .child(self.graph_source.clone()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(self.graph.clone())
                    .child(
                        h_flex()
                            .absolute()
                            .bottom_3()
                            .right_3()
                            .gap_1()
                            .child(
                                ghost_button("fit-view", "Fit", cx)
                                    .h_7()
                                    .text_xs()
                                    .bg(Theme::global(cx).tokens.colors.surface)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.graph.update(cx, |graph, cx| graph.fit_view(cx));
                                    })),
                            )
                            .child(
                                ghost_button("relayout", "Re-layout", cx)
                                    .h_7()
                                    .text_xs()
                                    .bg(Theme::global(cx).tokens.colors.surface)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.graph.update(cx, |graph, cx| graph.relayout(cx));
                                    })),
                            ),
                    ),
            )
            .children(show_results.then(|| self.render_query_bar(cx)))
            .children(
                self.results
                    .as_ref()
                    .filter(|_| show_results)
                    .map(|result| self.render_results(result, cx)),
            )
    }
}
