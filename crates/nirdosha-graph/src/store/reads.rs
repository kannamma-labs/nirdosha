use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

impl Graph {
    pub fn get(&self, kind: &str, key: &str, snapshot: Option<&str>) -> Result<Value> {
        self.check_access(false)?;
        let (r, view) = if let Some(token) = snapshot {
            self.snapshot(token)?
        } else {
            (self.raw_version()?.revision, "proposed".into())
        };
        let state = self.state_view(r, &view)?;
        let key = self.resolve_alias(key)?;
        let result = match kind {
            "node" => state
                .nodes
                .get(&key)
                .filter(|x| !x.deleted)
                .map(serde_json::to_value),
            "edge" => state
                .edges
                .get(&key)
                .filter(|x| !x.deleted)
                .map(serde_json::to_value),
            "gap" => state.gaps.get(&key).map(serde_json::to_value),
            _ => None,
        }
        .transpose()?
        .ok_or_else(|| Error::new("NOT_FOUND", &key))?;
        self.check_access(false)?;
        let mut v = self.raw_version()?;
        v.revision = r;
        Ok(json!({"graph":v,"view":view,"entity":result}))
    }
    pub fn resolve_alias(&self, key: &str) -> Result<String> {
        let mut q = self
            .conn
            .prepare("SELECT DISTINCT id FROM graph_aliases WHERE alias=?1")?;
        let ids = q
            .query_map([key], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        match ids.as_slice() {
            [] => Ok(key.into()),
            [id] => Ok(id.clone()),
            _ => Err(Error::new("AMBIGUOUS_ALIAS", key).details(json!({"candidates":ids}))),
        }
    }
    pub fn new_snapshot(&self, view: &str) -> Result<String> {
        self.check_access(false)?;
        if !["proposed", "accepted"].contains(&view) {
            return Err(Error::new("SCHEMA_INVALID", "Unknown read view"));
        }
        let v = self.raw_version()?;
        let key = id("snap")?;
        // Lease metadata is not an authoring mutation. Read-only access never migrates.
        let lease = Connection::open_with_flags(
            self.root.join(".nir/hi.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        configure(&lease, false)?;
        let count: u64 = lease.query_row(
            "SELECT COUNT(*) FROM graph_snapshots WHERE expires>=?1",
            [now()],
            |r| r.get(0),
        )?;
        if count >= 256 {
            return Err(Error::new("LIMIT_EXCEEDED", "Snapshot lease quota"));
        }
        lease.execute(
            "INSERT INTO graph_snapshots VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                key,
                self.access.grant_id,
                v.epoch,
                v.revision,
                view,
                now(),
                now() + 600
            ],
        )?;
        self.check_access(false)?;
        Ok(key)
    }
    pub(crate) fn snapshot(&self, key: &str) -> Result<(u64, String)> {
        self.check_access(false)?;
        let row: Option<(String, String, u64, String, u64)> = self
            .conn
            .query_row(
                "SELECT grant_id,epoch,revision,view,expires FROM graph_snapshots WHERE id=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((grant, epoch, r, view, expires)) = row else {
            return Err(Error::new("SNAPSHOT_EXPIRED", "Unknown snapshot"));
        };
        if grant != self.access.grant_id {
            return Err(Error::new(
                "UNAUTHORIZED",
                "Snapshot belongs to another grant",
            ));
        }
        if epoch != self.raw_version()?.epoch || expires < now() {
            return Err(Error::new("SNAPSHOT_EXPIRED", "Snapshot expired"));
        }
        Ok((r, view))
    }
    pub(crate) fn state_view(&self, r: u64, view: &str) -> Result<State> {
        if view == "accepted" {
            self.accepted_state_at(r)
        } else {
            self.load_state(Some(r))
        }
    }

    pub fn page(&self, args: &Value) -> Result<Value> {
        self.check_access(false)?;
        let requested_view = args["view"].as_str().unwrap_or("proposed");
        let (token, offset) = if let Some(cursor) = args["cursor"].as_str() {
            let (a, b) = cursor
                .rsplit_once(':')
                .ok_or_else(|| Error::new("SCHEMA_INVALID", "Bad page cursor"))?;
            (
                a.rsplit_once(':').ok_or_else(|| Error::new("SCHEMA_INVALID", "Missing cursor query binding"))?.0.to_owned(),
                b.parse::<usize>()
                    .map_err(|_| Error::new("SCHEMA_INVALID", "Bad cursor offset"))?,
            )
        } else {
            (
                args["snapshot"]
                    .as_str()
                    .map(str::to_owned)
                    .map(Ok)
                    .unwrap_or_else(|| self.new_snapshot(requested_view))?,
                0,
            )
        };
        let (r, view) = self.snapshot(&token)?;
        if args.get("view").is_some() && view != requested_view {
            return Err(Error::new("SCHEMA_INVALID", "View differs from snapshot"));
        }
        let s = self.state_view(r, &view)?;
        let include_deleted = args["include_deleted"].as_bool().unwrap_or(false);
        // Projection/query is part of the continuation checksum, preventing query drift.
        let filters = json!({"entity_type":args["entity_type"],"kind":args["kind"],"text":args["text"],"include_deleted":include_deleted});
        let filter_hash = hash::structured("query", &filters)?;
        if args["cursor"].as_str().is_some_and(|c| c.rsplit_once(':').and_then(|(p,_)|p.rsplit_once(':')).is_none_or(|(_,h)| h != filter_hash)) {
            return Err(Error::new(
                "SCHEMA_INVALID",
                "Continuation filter hash missing or changed",
            ));
        }
        let mut records = vec![];
        for e in s.edges.values().filter(|e| include_deleted || !e.deleted) {
            records.push(("edge", serde_json::to_value(e)?));
        }
        for g in s.gaps.values() {
            records.push(("gap", serde_json::to_value(g)?));
        }
        for n in s.nodes.values().filter(|n| include_deleted || !n.deleted) {
            records.push(("node", serde_json::to_value(n)?));
        }
        records.retain(|(t, v)| {
            args["entity_type"].as_str().is_none_or(|x| x == *t)
                && args["kind"].as_str().is_none_or(|x| v["kind"] == x)
                && args["text"]
                    .as_str()
                    .is_none_or(|x| v.to_string().to_lowercase().contains(&x.to_lowercase()))
        });
        let total = records.len();
        if offset > total {
            return Err(Error::new("SCHEMA_INVALID", "Cursor outside selection"));
        }
        let limit = args["limit"].as_u64().unwrap_or(200).clamp(1, 200) as usize;
        let mut nodes = vec![];
        let mut edges = vec![];
        let mut gaps = vec![];
        let mut bytes = 1024;
        let mut end = offset;
        for (t, v) in records.into_iter().skip(offset).take(limit) {
            let size = serde_json::to_vec(&v)?.len();
            if bytes + size > 512 * 1024 {
                break;
            }
            bytes += size;
            end += 1;
            match t {
                "node" => nodes.push(v),
                "edge" => edges.push(v),
                _ => gaps.push(v),
            }
        }
        self.check_access(false)?;
        let mut version = self.raw_version()?;
        version.revision = r;
        Ok(
            json!({"graph":version,"view":view,"nodes":nodes,"edges":edges,"gaps":gaps,"snapshot":token,"next_cursor":if end<total{Some(format!("{token}:{filter_hash}:{end}"))}else{None},"filter_hash":filter_hash,"page_complete":true,"selection_complete":end==total,"total_selected":total}),
        )
    }
    pub fn changes(&self, cursor: Option<&str>, limit: usize) -> Result<Value> {
        self.check_access(false)?;
        let v = self.raw_version()?;
        let after = if let Some(c) = cursor {
            let (e, r) = c
                .rsplit_once(':')
                .ok_or_else(|| Error::new("CURSOR_EXPIRED", "Invalid change cursor"))?;
            if e != v.epoch {
                return Err(Error::new("CURSOR_EXPIRED", "Epoch changed"));
            }
            r.parse::<u64>()
                .map_err(|_| Error::new("CURSOR_EXPIRED", "Invalid revision"))?
        } else {
            0
        };
        if after > v.revision {
            return Err(Error::new("CURSOR_EXPIRED", "Cursor beyond current head"));
        }
        let mut q=self.conn.prepare("SELECT revision,payload FROM graph_changes WHERE revision>?1 ORDER BY revision LIMIT ?2")?;
        let rows = q.query_map(params![after, limit.clamp(1, 100)], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut groups = vec![];
        let mut end = after;
        let mut bytes = 0;
        for row in rows {
            let (r, s) = row?;
            if bytes + s.len() > 512 * 1024 {
                if groups.is_empty() {
                    return Err(Error::new(
                        "LIMIT_EXCEEDED",
                        "Transaction requires snapshot retrieval",
                    )
                    .details(json!({"revision":r,"recovery":"graph_get_graph"})));
                }
                break;
            }
            bytes += s.len();
            end = r;
            groups.push(serde_json::from_str::<Value>(&s)?);
        }
        self.check_access(false)?;
        Ok(
            json!({"graph":v,"transactions":groups,"next_cursor":format!("{}:{end}",v.epoch),"selection_complete":end==v.revision}),
        )
    }
    pub fn neighbors(&self, args: &Value) -> Result<Value> {
        self.check_access(false)?;
        let token = args["snapshot"]
            .as_str()
            .map(str::to_owned)
            .map(Ok)
            .unwrap_or_else(|| self.new_snapshot("proposed"))?;
        let (r, view) = self.snapshot(&token)?;
        let s = self.state_view(r, &view)?;
        let id = self.resolve_alias(mutations::required(args, "node_id")?)?;
        if !s.nodes.get(&id).is_some_and(|n| !n.deleted) {
            return Err(Error::new("NOT_FOUND", id));
        }
        let depth = args["depth"].as_u64().unwrap_or(1).min(5);
        let direction = args["direction"].as_str().unwrap_or("both");
        if !["both", "out", "in"].contains(&direction) {
            return Err(Error::new("SCHEMA_INVALID", "Invalid traversal direction"));
        }
        let mut seen = BTreeSet::from([id.clone()]);
        let mut frontier = BTreeSet::from([id]);
        let mut edge_ids = BTreeSet::new();
        let mut truncated = false;
        for _ in 0..depth {
            let mut next = BTreeSet::new();
            for e in s.edges.values().filter(|e| !e.deleted) {
                if args["edge_kinds"]
                    .as_array()
                    .is_some_and(|xs| !xs.contains(&json!(e.kind)))
                {
                    continue;
                }
                let target = if direction != "in" && frontier.contains(&e.src) {
                    Some(&e.dst)
                } else if direction != "out" && frontier.contains(&e.dst) {
                    Some(&e.src)
                } else {
                    None
                };
                if let Some(t) = target {
                    if !seen.contains(t) && seen.len() >= 1000 {
                        truncated = true;
                        continue;
                    }
                    edge_ids.insert(e.id.clone());
                    if seen.insert(t.clone()) {
                        next.insert(t.clone());
                    }
                }
            }
            frontier = next;
        }
        let nodes: Vec<_> = seen.iter().filter_map(|id| s.nodes.get(id)).collect();
        let edges: Vec<_> = edge_ids.iter().filter_map(|id| s.edges.get(id)).collect();
        let result = json!({"snapshot":token,"revision":r,"nodes":nodes,"edges":edges,"truncated":truncated,"boundary_ids":frontier});
        if serde_json::to_vec(&result)?.len() > 512 * 1024 {
            return Err(Error::new(
                "LIMIT_EXCEEDED",
                "Neighborhood too large; use a smaller depth",
            ));
        }
        self.check_access(false)?;
        Ok(result)
    }
    pub fn export(&self, snapshot: Option<&str>) -> Result<Value> {
        self.check_access(false)?;
        let token = snapshot
            .map(str::to_owned)
            .map(Ok)
            .unwrap_or_else(|| self.new_snapshot("proposed"))?;
        let (r, view) = self.snapshot(&token)?;
        let s = self.state_view(r, &view)?;
        let mut v = self.raw_version()?;
        v.revision = r;
        let content = serde_json::to_vec(
            &json!({"graph":v,"view":view,"nodes":s.nodes.values().collect::<Vec<_>>(),"edges":s.edges.values().collect::<Vec<_>>(),"gaps":s.gaps.values().collect::<Vec<_>>()}),
        )?;
        // Exports are read artifacts in host-local lease storage, not source mutations.
        let dir = self.host.join("exports");
        std::fs::create_dir_all(&dir)?;
        let h = hash::bytes(&content);
        let artifact = format!("{token}_{h}");
        let path = dir.join(&artifact);
        std::fs::write(path, &content)?;
        self.check_access(false)?;
        Ok(
            json!({"snapshot":token,"artifact_id":artifact,"size":content.len(),"hash":h,"resource_uri":format!("nirdosha-hi://{}/artifacts/{artifact}/manifest",v.project_id)}),
        )
    }
    pub fn read_artifact(&self, key: &str, offset: usize, length: usize) -> Result<Value> {
        self.check_access(false)?;
        let data = if key.starts_with("snap_") {
            let (split, hash) = key
                .rsplit_once('_')
                .ok_or_else(|| Error::new("SCHEMA_INVALID", "Invalid artifact ID"))?;
            self.snapshot(split)?;
            if !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return Err(Error::new("SCHEMA_INVALID", "Invalid artifact ID"));
            }
            let bytes = std::fs::read(self.host.join("exports").join(key))?;
            if hash::bytes(&bytes) != hash {
                return Err(Error::new("DATABASE_INVALID", "Export checksum mismatch"));
            }
            bytes
        } else {
            self.blob(key)?
        };
        if offset > data.len() {
            return Err(Error::new("SCHEMA_INVALID", "Offset exceeds artifact size"));
        }
        let end = offset
            .saturating_add(length.clamp(1, 65536))
            .min(data.len());
        self.check_access(false)?;
        Ok(
            json!({"artifact_id":key,"offset":offset,"next_offset":end,"size":data.len(),"hash":hash::bytes(&data),"bytes_base64":STANDARD.encode(&data[offset..end]),"complete":end==data.len()}),
        )
    }
}
