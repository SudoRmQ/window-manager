//! i3's IPC protocol: every message is `"i3-ipc"` + payload length (u32) + type (u32) +
//! payload, integers in native byte order. Replies carry the same type. Payloads are JSON.
//! See <https://i3wm.org/docs/ipc.html>. Transport (Unix socket / named pipe) lives in the
//! binary; this module only encodes, decodes and answers queries.

use crate::tree::{Kind, NodeId, Tree};
use serde_json::{json, Value};
use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 6] = b"i3-ipc";

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_OUTPUTS: u32 = 3;
pub const GET_TREE: u32 = 4;
pub const GET_VERSION: u32 = 7;

/// Message type for an `i3-msg -t` style name.
pub fn type_from_name(name: &str) -> Option<u32> {
    Some(match name {
        "command" | "run_command" => RUN_COMMAND,
        "get_workspaces" => GET_WORKSPACES,
        "subscribe" => SUBSCRIBE,
        "get_outputs" => GET_OUTPUTS,
        "get_tree" => GET_TREE,
        "get_version" => GET_VERSION,
        _ => return None,
    })
}

pub fn write_message(w: &mut impl Write, ty: u32, payload: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(14 + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    buf.extend_from_slice(&ty.to_ne_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

pub fn read_message(r: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    r.read_exact(&mut header)?;
    if &header[..6] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad magic"));
    }
    let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
    let ty = u32::from_ne_bytes(header[10..14].try_into().unwrap());
    if len > 16 << 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((ty, payload))
}

fn rect(r: crate::geometry::Rect) -> Value {
    json!({"x": r.x, "y": r.y, "width": r.w, "height": r.h})
}

fn node_json(t: &Tree, id: NodeId) -> Value {
    let n = t.node(id);
    let (ty, name, window) = match &n.kind {
        Kind::Root => ("root", Some("root".to_string()), None),
        Kind::Output { name } => ("output", Some(name.clone()), None),
        Kind::Workspace { name } => ("workspace", Some(name.clone()), None),
        Kind::Split => ("con", None, None),
        Kind::Window(w) => (
            if t.is_floating(id) {
                "floating_con"
            } else {
                "con"
            },
            None,
            Some(w.0),
        ),
    };
    json!({
        "id": id,
        "type": ty,
        "name": name,
        "window": window,
        "layout": n.layout.name(),
        "orientation": match n.layout.orientation() {
            crate::tree::Orientation::Horizontal => "horizontal",
            crate::tree::Orientation::Vertical => "vertical",
        },
        "percent": if matches!(n.kind, Kind::Split | Kind::Window(_)) { json!(n.percent) } else { Value::Null },
        "rect": rect(n.rect),
        "focused": id == t.focused,
        "focus": n.focus,
        "fullscreen_mode": n.fullscreen as u8,
        "nodes": n.children.iter().map(|&c| node_json(t, c)).collect::<Vec<_>>(),
        "floating_nodes": n.floating.iter().map(|&c| node_json(t, c)).collect::<Vec<_>>(),
    })
}

/// Answer a read-only request.
pub fn query(t: &Tree, ty: u32) -> String {
    let cur = t.current_workspace();
    let v = match ty {
        GET_TREE => node_json(t, t.root),
        GET_WORKSPACES => Value::Array(
            t.workspaces()
                .into_iter()
                .map(|ws| {
                    let name = t.workspace_name(ws);
                    let num: i64 = name.parse().unwrap_or(-1);
                    let output = t.node(ws).parent.map(|o| t.workspace_name(o).to_string());
                    json!({
                        "id": ws, "num": num, "name": name,
                        "visible": t.is_visible_workspace(ws), "focused": ws == cur,
                        "urgent": false, "rect": rect(t.node(ws).rect), "output": output,
                    })
                })
                .collect(),
        ),
        GET_OUTPUTS => Value::Array(
            t.outputs()
                .into_iter()
                .map(|o| {
                    let ws = t
                        .visible_workspace(o)
                        .map(|w| t.workspace_name(w).to_string());
                    json!({
                        "name": t.workspace_name(o), "active": true, "primary": true,
                        "rect": rect(t.node(o).rect), "current_workspace": ws,
                    })
                })
                .collect(),
        ),
        GET_VERSION => json!({
            "major": 0, "minor": 1, "patch": 0,
            "human_readable": concat!("tiler ", env!("CARGO_PKG_VERSION"), " (i3 IPC compatible subset)"),
        }),
        SUBSCRIBE => json!({"success": false, "error": "events are not supported yet"}),
        _ => json!({"success": false, "error": format!("unsupported message type {ty}")}),
    };
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{geometry::Rect, tree::TreeOptions, WindowId};

    #[test]
    fn round_trip() {
        let mut buf = vec![];
        write_message(&mut buf, GET_TREE, b"").unwrap();
        write_message(&mut buf, RUN_COMMAND, b"focus left").unwrap();
        let mut r = &buf[..];
        assert_eq!(read_message(&mut r).unwrap(), (GET_TREE, vec![]));
        assert_eq!(
            read_message(&mut r).unwrap(),
            (RUN_COMMAND, b"focus left".to_vec())
        );
    }

    #[test]
    fn tree_and_workspaces_json() {
        let mut t = Tree::new(
            &[("primary".into(), Rect::new(0, 0, 100, 100))],
            TreeOptions::default(),
        );
        t.manage(WindowId(42));
        let tree: Value = serde_json::from_str(&query(&t, GET_TREE)).unwrap();
        let win = &tree["nodes"][0]["nodes"][0]["nodes"][0];
        assert_eq!(win["window"], 42);
        assert_eq!(win["focused"], true);
        let ws: Value = serde_json::from_str(&query(&t, GET_WORKSPACES)).unwrap();
        assert_eq!(ws[0]["name"], "1");
        assert_eq!(ws[0]["focused"], true);
    }
}
