// Built-in MCP server (`murmur --mcp`): read-only Model Context Protocol
// access to the app's SQLite database over stdio, replacing the old
// `backend/mcp_server/` Python implementation.
//
// MCP clients (Claude Desktop / Claude Code / Cursor / …) spawn the Murmur
// executable with the `--mcp` flag; it speaks newline-delimited JSON-RPC 2.0
// on stdin/stdout (the MCP stdio transport) and exits when stdin closes. The
// GUI does not need to be running — and when it is, the read-only WAL reader
// coexists safely. The single-instance guard is bypassed for this mode in
// `main.rs`.
//
// The protocol surface is deliberately minimal (tools only, hand-rolled):
// initialize / ping / tools/list / tools/call. Logs go to stderr only —
// stdout is reserved for protocol frames.

pub mod tools;

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// Resolve the Murmur database path the same way the Tauri app does
/// (`app_data_dir` = platform data dir + bundle identifier), without booting
/// Tauri. Overridable via `MURMUR_DB_PATH` (or the legacy `MEETILY_DB_PATH`)
/// or `--db <path>`.
pub fn default_db_path() -> Result<PathBuf> {
    const IDENTIFIER: &str = "com.murmur.app";
    // Pre-rename installs kept their data under the old Meetily identifier;
    // the GUI migrates that directory on first launch, but `murmur --mcp` can
    // run before the GUI ever starts, so fall back to the legacy location.
    const LEGACY_IDENTIFIER: &str = "com.meetily.ai";

    let base = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var("APPDATA").map_err(|_| anyhow!("APPDATA is not set"))?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var("HOME").map_err(|_| anyhow!("HOME is not set"))?)
            .join("Library")
            .join("Application Support")
    } else {
        match std::env::var("XDG_DATA_HOME") {
            Ok(xdg) if !xdg.trim().is_empty() => PathBuf::from(xdg),
            _ => PathBuf::from(std::env::var("HOME").map_err(|_| anyhow!("HOME is not set"))?)
                .join(".local")
                .join("share"),
        }
    };

    let current = base.join(IDENTIFIER).join("meeting_minutes.sqlite");
    if !current.exists() {
        let legacy = base.join(LEGACY_IDENTIFIER).join("meeting_minutes.sqlite");
        if legacy.exists() {
            return Ok(legacy);
        }
    }
    Ok(current)
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "list_meetings",
            "description": "List Murmur meetings, most recent first. Returns meeting_id values usable with the other tools, plus whether a summary exists.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Maximum number of meetings to return (default 50)."}
                }
            }
        },
        {
            "name": "get_transcript",
            "description": "Get the full transcript of a Murmur meeting, with speaker tags and timestamps when available.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "The meeting's id (from list_meetings)."}
                },
                "required": ["meeting_id"]
            }
        },
        {
            "name": "get_summary",
            "description": "Get the AI-generated summary of a Murmur meeting as markdown, or a status note if no summary exists.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "The meeting's id (from list_meetings)."}
                },
                "required": ["meeting_id"]
            }
        },
        {
            "name": "get_meeting",
            "description": "Get everything about a meeting: metadata, summary, and full transcript in one call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "The meeting's id (from list_meetings)."}
                },
                "required": ["meeting_id"]
            }
        },
        {
            "name": "search_transcripts",
            "description": "Search across all meeting transcripts for a word or phrase (case-insensitive substring match).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Text to search for."},
                    "limit": {"type": "integer", "description": "Maximum number of matches (default 20)."}
                },
                "required": ["query"]
            }
        }
    ])
}

async fn call_tool(pool: &SqlitePool, name: &str, args: &Value) -> Result<String> {
    let str_arg = |key: &str| -> Result<String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Missing required argument '{key}'"))
    };
    let int_arg = |key: &str, default: i64| -> i64 {
        args.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    };

    match name {
        "list_meetings" => tools::list_meetings(pool, int_arg("limit", 50)).await,
        "get_transcript" => tools::get_transcript(pool, &str_arg("meeting_id")?).await,
        "get_summary" => tools::get_summary(pool, &str_arg("meeting_id")?).await,
        "get_meeting" => tools::get_meeting(pool, &str_arg("meeting_id")?).await,
        "search_transcripts" => {
            tools::search_transcripts(pool, &str_arg("query")?, int_arg("limit", 20)).await
        }
        other => Err(anyhow!("Unknown tool: {other}")),
    }
}

fn handle_initialize(params: &Value) -> Value {
    // Echo the client's requested protocol version (all versions we care
    // about share this tools-only surface); fall back to a recent default.
    let version = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "murmur",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

fn text_tool_result(text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

/// Run the stdio MCP server until stdin closes.
pub async fn run_stdio(db_path: PathBuf) -> Result<()> {
    log::info!("Murmur MCP server starting (db: {})", db_path.display());
    let pool = tools::open_readonly_pool(&db_path).await?;

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Ignoring non-JSON input line: {e}");
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        // Notifications (no id) never get a response.
        if id.is_none() {
            continue;
        }
        let id = id.unwrap();

        let response = match method {
            "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": handle_initialize(&params)}),
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => {
                json!({"jsonrpc": "2.0", "id": id, "result": { "tools": tool_definitions() }})
            }
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let result = match call_tool(&pool, name, &args).await {
                    Ok(text) => text_tool_result(text, false),
                    Err(e) => text_tool_result(format!("Error: {e:#}"), true),
                };
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            }
            other => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {other}") }
            }),
        };

        let mut frame = serde_json::to_vec(&response)?;
        frame.push(b'\n');
        stdout.write_all(&frame).await?;
        stdout.flush().await?;
    }

    log::info!("stdin closed — MCP server shutting down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_echoes_client_protocol_version() {
        let result = handle_initialize(&json!({"protocolVersion": "2024-11-05"}));
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "murmur");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_defaults_protocol_version_when_absent() {
        let result = handle_initialize(&json!({}));
        assert_eq!(result["protocolVersion"], DEFAULT_PROTOCOL_VERSION);
    }

    #[test]
    fn tool_definitions_cover_the_five_tools_with_schemas() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["list_meetings", "get_transcript", "get_summary", "get_meeting", "search_transcripts"]
        );
        for tool in tools.as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().unwrap().len() > 20);
        }
    }
}
