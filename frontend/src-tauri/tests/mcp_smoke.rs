//! End-to-end smoke test for the built-in MCP server: builds a migrated fixture
//! DB, spawns the real `murmur --mcp --db <fixture>` binary, and drives it over
//! stdio (newline-delimited JSON-RPC) to assert initialize / tools/list /
//! tools/call work against real data. This is the only test that exercises the
//! actual shipped binary + argument parsing + stdio framing end to end.
//!
//! Heavy: it forces a build of the full `murmur` binary, so CI runs it in its own
//! job. stdout is the protocol channel; the server logs to stderr (nulled here).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Send one JSON-RPC request line and read exactly one JSON response line.
fn rpc(stdin: &mut std::process::ChildStdin, reader: &mut impl BufRead, req: Value) -> Value {
    let line = serde_json::to_string(&req).unwrap();
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();

    let mut resp = String::new();
    let n = reader.read_line(&mut resp).unwrap();
    assert!(n > 0, "MCP server closed stdout before responding to {req}");
    serde_json::from_str(&resp).unwrap_or_else(|e| panic!("non-JSON response {resp:?}: {e}"))
}

#[tokio::test]
async fn mcp_server_responds_over_stdio() {
    // 1. Build a migrated fixture DB with one meeting + transcript.
    let dir = std::env::temp_dir().join(format!("murmur_mcp_smoke_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("fixture.sqlite");
    let _ = std::fs::remove_file(&db_path);
    {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','Smoke Test Meeting','2026-07-13','2026-07-13')")
            .execute(&pool).await.unwrap();
        // The transcript INSERT fires the FTS trigger, so search must find it.
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES ('t1','m1','we discussed the quarterly budget','[00:00]')")
            .execute(&pool).await.unwrap();
        pool.close().await;
    }

    // 2. Spawn the real binary against the fixture DB.
    let mut child = Command::new(env!("CARGO_BIN_EXE_murmur"))
        .arg("--mcp")
        .arg("--db")
        .arg(&db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn murmur --mcp");

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    // 3. initialize
    let init = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "murmur", "init: {init}");
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");

    // 4. tools/list
    let tools = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    );
    let names: Vec<String> = tools["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect();
    for expected in ["list_meetings", "get_transcript", "get_summary", "get_meeting", "search_transcripts"] {
        assert!(names.iter().any(|n| n == expected), "missing tool {expected} in {names:?}");
    }

    // 5. tools/call list_meetings → sees the fixture meeting
    let listed = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_meetings","arguments":{}}}),
    );
    let listed_text = listed["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(listed_text.contains("Smoke Test Meeting"), "list_meetings: {listed_text}");
    assert_eq!(listed["result"]["isError"], false);

    // 6. tools/call search_transcripts → FTS finds the transcript
    let searched = rpc(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_transcripts","arguments":{"query":"budget"}}}),
    );
    let searched_text = searched["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(searched_text.contains("`m1`"), "search_transcripts: {searched_text}");

    // 7. Close stdin → the server's read loop ends and it exits cleanly.
    drop(stdin);
    let status = child.wait().expect("wait for murmur --mcp to exit");
    assert!(status.success(), "MCP server exited with {status}");

    let _ = std::fs::remove_dir_all(&dir);
}
