//! Project-bound transport for RFC 0021. Authorization is selected by the host CLI,
//! never by tool arguments. Notifications are hints; clients recover via cursors.
use nirdosha_graph::{Error, Graph, Result, store::Access};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};

pub struct Options {
    pub project: Option<PathBuf>,
    pub state: PathBuf,
    pub access: Access,
}
impl Options {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut args = args;
        let mut project = None;
        let mut mode = "read".to_owned();
        let mut state = std::env::var_os("NIRDOSHA_GRAPH_STATE_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_STATE_HOME").map(|p| PathBuf::from(p).join("nirdosha"))
            })
            .or_else(|| {
                std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state/nirdosha"))
            })
            .ok_or_else(|| Error::new("SCHEMA_INVALID", "Specify --state-dir"))?;
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| Error::new("SCHEMA_INVALID", format!("Missing value for {flag}")))?;
            match flag.as_str() {
                "--project" => project = Some(value.into()),
                "--state-dir" => state = value.into(),
                "--graph-access" => mode = value,
                _ => {
                    return Err(Error::new(
                        "SCHEMA_INVALID",
                        format!("Unknown option {flag}"),
                    ));
                }
            }
        }
        let access = match mode.as_str() {
            "read" => Access::read("local"),
            "author" => Access::author("local"),
            "reviewer" => Access::reviewer("local"),
            _ => {
                return Err(Error::new(
                    "SCHEMA_INVALID",
                    "--graph-access must be read, author or reviewer",
                ));
            }
        };
        Ok(Self {
            project,
            state,
            access,
        })
    }
    pub fn open(&self) -> Result<Option<Graph>> {
        self.project
            .as_ref()
            .map(|p| Graph::open(p, &self.state, self.access.clone()))
            .transpose()
    }
}

pub struct Session {
    pub graph: Graph,
    subscriptions: BTreeSet<String>,
    last_revision: u64,
}
impl Session {
    pub fn new(graph: Graph) -> Result<Self> {
        let last_revision = graph.version()?.revision;
        Ok(Self {
            graph,
            subscriptions: BTreeSet::new(),
            last_revision,
        })
    }
    pub fn dispatch(&mut self, method: &str, params: &Value) -> Option<Value> {
        let result: Result<Value> = match method {
            "tools/list" => {
                let mut result = crate::mcp_tools::tools_list();
                result["tools"].as_array_mut().unwrap().extend(
                    nirdosha_graph::mcp::tools(&self.graph)["tools"]
                        .as_array()
                        .unwrap()
                        .clone(),
                );
                Ok(result)
            }
            "tools/call"
                if params["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("graph_")) =>
            {
                Ok(nirdosha_graph::mcp::call(
                    &self.graph,
                    params["name"].as_str().unwrap(),
                    &params["arguments"],
                ))
            }
            "resources/list" => nirdosha_graph::mcp::resources(&self.graph),
            "resources/templates/list" => nirdosha_graph::mcp::templates(&self.graph),
            "resources/read" => nirdosha_graph::mcp::resource_read(
                &self.graph,
                params["uri"].as_str().unwrap_or(""),
            ),
            "resources/subscribe" => {
                let uri = params["uri"].as_str().unwrap_or("");
                match nirdosha_graph::mcp::resource_read(&self.graph, uri) {
                    Ok(_) => {
                        self.subscriptions.insert(uri.into());
                        Ok(json!({}))
                    }
                    Err(e) => Err(e),
                }
            }
            "resources/unsubscribe" => {
                self.subscriptions
                    .remove(params["uri"].as_str().unwrap_or(""));
                Ok(json!({}))
            }
            _ => return None,
        };
        Some(match result {
            Ok(v) => json!({"result":v}),
            Err(e) => json!({"error":{"code":-32000,"message":e.message,"data":e.envelope()}}),
        })
    }
    pub fn notifications(&mut self) -> Vec<Value> {
        let Ok(v) = self.graph.version() else {
            self.subscriptions.clear();
            return vec![];
        };
        if v.revision == self.last_revision {
            return vec![];
        }
        self.last_revision = v.revision;
        self.subscriptions.iter().map(|uri|json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":uri}})).collect()
    }
}

/// Detection must neither initialize nor migrate the database.
pub fn is_typed(root: &std::path::Path) -> bool {
    rusqlite::Connection::open_with_flags(
        root.join(".nir/hi.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .and_then(|c| {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='graph_meta')",
            [],
            |r| r.get(0),
        )
    })
    .unwrap_or(false)
}
