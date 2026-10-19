use std::collections::BTreeMap;

use gpui::SharedString;
use gpui_graph::{Edge, GraphData, Node};
use hawdb::{Database, QueryOutput, Value};

use crate::{ResultPreview, preview_text, validate_query};

pub const GRAPH_QUERY: &str = "MATCH (a)-[r]->(b) RETURN a, r, b";
pub const SQL_QUERY: &str = "SELECT * FROM sightings ORDER BY id";
pub const MAX_HISTORY: usize = 40;
const MAX_GRAPH_ITEMS: usize = 2000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Cypher,
    Sql,
}

impl Language {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cypher => "CYPHER",
            Self::Sql => "SQL",
        }
    }
}

pub struct Entry {
    pub query: SharedString,
    pub language: Language,
    pub result: Option<ResultPreview>,
    pub error: Option<SharedString>,
    pub graph: Option<GraphData>,
    pub graph_note: SharedString,
}

pub fn execute(database: &mut Database, query: &str, language: Language) -> Entry {
    let output = validate_query(query).map_err(str::to_owned).and_then(|_| {
        match language {
            Language::Cypher => database.query(query),
            Language::Sql => database.query_sql(query),
        }
        .map_err(|error| error.to_string())
    });
    let mut entry = Entry {
        query: query
            .chars()
            .take(crate::MAX_QUERY_BYTES)
            .collect::<String>()
            .into(),
        language,
        result: None,
        error: None,
        graph: None,
        graph_note: "Table result · return nodes / relationships to draw a graph".into(),
    };
    match output {
        Ok(output) => {
            entry.result = Some(ResultPreview::from_output(&output));
            match graph_from_output(&output) {
                Ok(Some(graph)) => {
                    entry.graph_note = format!(
                        "{} nodes · {} relationships · returned objects only",
                        graph.nodes.len(),
                        graph.edges.len()
                    )
                    .into();
                    entry.graph = Some(graph);
                }
                Ok(None) => {}
                Err(error) => entry.graph_note = error.into(),
            }
        }
        Err(error) => entry.error = Some(error.into()),
    }
    entry
}

#[derive(Default)]
pub struct History {
    pub entries: Vec<Entry>,
    cursor: Option<usize>,
    draft: String,
    draft_language: Language,
}

impl History {
    pub fn push(&mut self, entry: Entry) {
        if self.entries.len() == MAX_HISTORY {
            self.entries.remove(0);
        }
        self.entries.push(entry);
        self.cursor = None;
        self.draft.clear();
    }

    pub fn previous(&mut self, draft: &str, language: Language) -> Option<(String, Language)> {
        if self.entries.is_empty() {
            return None;
        }
        let index = match self.cursor {
            None => {
                self.draft = draft.to_owned();
                self.draft_language = language;
                self.entries.len() - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.cursor = Some(index);
        let entry = &self.entries[index];
        Some((entry.query.to_string(), entry.language))
    }

    pub fn next(&mut self) -> Option<(String, Language)> {
        let index = self.cursor? + 1;
        if index >= self.entries.len() {
            self.cursor = None;
            Some((self.draft.clone(), self.draft_language))
        } else {
            self.cursor = Some(index);
            let entry = &self.entries[index];
            Some((entry.query.to_string(), entry.language))
        }
    }
}

pub fn seed_sql(database: &mut Database) -> Result<(), String> {
    database.query_sql("CREATE TABLE sightings (id BIGINT PRIMARY KEY, species TEXT, location TEXT, count BIGINT)")
        .map_err(|error| error.to_string())?;
    database.query_sql("INSERT INTO sightings (id, species, location, count) VALUES (1, 'Red-tailed hawk', 'North Ridge', 3), (2, 'Peregrine falcon', 'River Bend', 2), (3, 'Red-tailed hawk', 'River Bend', 1)")
        .map_err(|error| error.to_string())?;
    Ok(())
}

// Use HawDB's actual returned identity and labels, not a user-defined `id`.
// This also supports nodes such as (:Hawk {name: 'Aurelia'}) without an id property.
pub fn graph_from_output(output: &QueryOutput) -> Result<Option<GraphData>, String> {
    let mut nodes = BTreeMap::<i64, Node>::new();
    let mut edges = BTreeMap::<i64, (i64, i64, Edge)>::new();
    for row in output.value_rows() {
        for value in row {
            collect(value, &mut nodes, &mut edges, 0)?;
        }
    }
    if nodes.is_empty() && edges.is_empty() {
        return Ok(None);
    }
    for (source, target, _) in edges.values() {
        for id in [source, target] {
            nodes.entry(*id).or_insert_with(|| {
                Node::new(id.to_string(), format!("Node #{id}"), "Endpoint").property(
                    "note",
                    "Return this node in the query to see its properties.",
                )
            });
        }
    }
    if nodes.len() + edges.len() > MAX_GRAPH_ITEMS {
        return Err("Graph exceeds 2000 items. Add LIMIT to your query.".into());
    }
    Ok(Some(GraphData::new(
        nodes.into_values().collect(),
        edges.into_values().map(|(_, _, edge)| edge).collect(),
    )))
}

type Nodes = BTreeMap<i64, Node>;
type Edges = BTreeMap<i64, (i64, i64, Edge)>;

fn collect(
    value: &Value,
    nodes: &mut Nodes,
    edges: &mut Edges,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Graph nesting exceeds 32 levels.".into());
    }
    match value {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_id") {
                if let (Some(Value::Int(source)), Some(Value::Int(target)), Some(kind)) =
                    (map.get("source_id"), map.get("target_id"), map.get("type"))
                {
                    let label = map.get("kind").unwrap_or(kind).to_string();
                    edges.entry(*id).or_insert((
                        *source,
                        *target,
                        Edge::new(source.to_string(), target.to_string(), label),
                    ));
                } else if let Some(Value::List(labels)) = map.get("labels") {
                    let label = map
                        .get("name")
                        .or_else(|| map.get("id"))
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("Node #{id}"));
                    let category = map
                        .get("kind")
                        .or_else(|| labels.first())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "Node".into());
                    let mut node = Node::new(id.to_string(), label, category);
                    for (key, value) in map {
                        if key != "_id" {
                            node = node.property(key.clone(), preview_text(&value.to_string()));
                        }
                    }
                    nodes.entry(*id).or_insert(node);
                } else {
                    for value in map.values() {
                        collect(value, nodes, edges, depth + 1)?;
                    }
                }
            } else {
                for value in map.values() {
                    collect(value, nodes, edges, depth + 1)?;
                }
            }
        }
        Value::List(values) => {
            for value in values {
                collect(value, nodes, edges, depth + 1)?;
            }
        }
        _ => {}
    }
    if nodes.len() + edges.len() > MAX_GRAPH_ITEMS {
        return Err("Graph exceeds 2000 items. Add LIMIT to your query.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_graph_is_filtered_deduplicated_and_keeps_parallel_edges() {
        let mut db = Database::new();
        crate::seed_sample(&mut db).unwrap();
        let entry = execute(
            &mut db,
            "MATCH (a {id: 'hawdb'})-[r]->(b) RETURN a, r, b",
            Language::Cypher,
        );
        assert!(entry.error.is_none(), "{:?}", entry.error);
        let graph = entry.graph.unwrap();
        assert_eq!(graph.nodes.len(), 7);
        assert_eq!(graph.edges.len(), 6);
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.label.as_ref() == "Reasoning agent")
        );
        db.query("MATCH (a {id: 'hawdb'}), (b {id: 'memory'}) CREATE (a)-[:RELATED]->(b)")
            .unwrap();
        let graph = execute(
            &mut db,
            "MATCH (a {id: 'hawdb'})-[r]->(b) RETURN a, r, b",
            Language::Cypher,
        )
        .graph
        .unwrap();
        assert_eq!(graph.nodes.len(), 7);
        assert_eq!(graph.edges.len(), 7);
    }

    #[test]
    fn node_objects_without_custom_ids_and_relationship_only_results_are_drawable() {
        let mut db = Database::new();
        db.query("CREATE (:Hawk {name: 'Aurelia'})").unwrap();
        db.query("CREATE (:Territory {name: 'North Ridge'})")
            .unwrap();
        db.query("MATCH (a:Hawk), (b:Territory) CREATE (a)-[:HUNTS_IN]->(b)")
            .unwrap();
        let graph = execute(&mut db, "MATCH (n:Hawk) RETURN n", Language::Cypher)
            .graph
            .unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].label.as_ref(), "Aurelia");
        assert_eq!(graph.nodes[0].category.as_ref(), "Hawk");
        let graph = execute(&mut db, "MATCH (a)-[r]->(b) RETURN r", Language::Cypher)
            .graph
            .unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].label.as_ref(), "HUNTS_IN");
    }

    #[test]
    fn sql_uses_the_sql_engine_and_scalar_results_do_not_become_graphs() {
        let mut db = Database::new();
        seed_sql(&mut db).unwrap();
        let entry = execute(&mut db, SQL_QUERY, Language::Sql);
        assert!(entry.error.is_none(), "{:?}", entry.error);
        assert_eq!(entry.result.unwrap().total_rows, 3);
        assert!(entry.graph.is_none());
        let entry = execute(&mut db, SQL_QUERY, Language::Cypher);
        assert!(entry.error.is_some());
        db.query("CREATE (:Hawk {name: 'Aurelia'})").unwrap();
        let scalar = execute(&mut db, "MATCH (h:Hawk) RETURN h.name", Language::Cypher);
        assert!(scalar.error.is_none());
        assert_eq!(scalar.result.unwrap().total_rows, 1);
        assert!(scalar.graph.is_none());
    }

    #[test]
    fn failed_commands_are_retained_and_history_restores_unsent_draft_and_language() {
        let mut db = Database::new();
        let mut history = History::default();
        history.push(execute(&mut db, "bad query", Language::Cypher));
        history.push(execute(&mut db, "SELECT 42 AS answer", Language::Sql));
        assert!(history.entries[0].error.is_some());
        assert_eq!(
            history.previous("unfinished", Language::Cypher).unwrap(),
            ("SELECT 42 AS answer".into(), Language::Sql)
        );
        assert_eq!(history.previous("", Language::Sql).unwrap().0, "bad query");
        assert_eq!(history.next().unwrap().1, Language::Sql);
        assert_eq!(
            history.next().unwrap(),
            ("unfinished".into(), Language::Cypher)
        );
        assert!(history.next().is_none());
        for _ in 0..MAX_HISTORY {
            history.push(execute(&mut db, "SELECT 1 AS value", Language::Sql));
        }
        assert_eq!(history.entries.len(), MAX_HISTORY);
        assert!(history.entries.iter().all(|entry| entry.error.is_none()));
    }

    #[test]
    fn graph_snapshots_are_immutable_and_do_not_rerun_writes() {
        let mut db = Database::new();
        execute(
            &mut db,
            "CREATE (:Hawk {name: 'Aurelia'})",
            Language::Cypher,
        );
        let entry = execute(&mut db, "MATCH (n) RETURN n", Language::Cypher);
        execute(&mut db, "MATCH (n) DETACH DELETE n", Language::Cypher);
        assert_eq!(entry.graph.unwrap().nodes.len(), 1);
        assert!(
            execute(&mut db, "MATCH (n) RETURN n", Language::Cypher)
                .graph
                .is_none()
        );
    }

    #[test]
    fn nested_graph_values_respect_graph_and_depth_limits() {
        let mut nodes = Nodes::new();
        let mut edges = Edges::new();
        let node = |id| {
            Value::Map(BTreeMap::from([
                ("_id".into(), Value::Int(id)),
                (
                    "labels".into(),
                    Value::List(vec![Value::String("Hawk".into())]),
                ),
            ]))
        };
        collect(
            &Value::List(vec![node(1), node(1)]),
            &mut nodes,
            &mut edges,
            0,
        )
        .unwrap();
        assert_eq!(nodes.len(), 1);
        for id in 2..=MAX_GRAPH_ITEMS as i64 {
            collect(&node(id), &mut nodes, &mut edges, 0).unwrap();
        }
        assert!(collect(&node(MAX_GRAPH_ITEMS as i64 + 1), &mut nodes, &mut edges, 0).is_err());
        let nested = (0..34).fold(node(1), |value, _| Value::List(vec![value]));
        assert!(collect(&nested, &mut Nodes::new(), &mut Edges::new(), 0).is_err());
    }
}
