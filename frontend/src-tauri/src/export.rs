use log::{error, info};
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use crate::database::models::Transcript;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::summary::SummaryProcessesRepository;
use crate::state::AppState;

#[tauri::command]
pub async fn export_meeting_markdown<R: Runtime>(
    app: AppHandle<R>,
    content: String,
    suggested_filename: String,
) -> Result<Option<String>, String> {
    info!(
        "export_meeting_markdown: opening save dialog (suggested filename: {})",
        suggested_filename
    );

    let app_clone = app.clone();
    let chosen = tokio::task::spawn_blocking(move || {
        app_clone
            .dialog()
            .file()
            .add_filter("Markdown", &["md"])
            .set_file_name(&suggested_filename)
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("Save dialog task failed: {e}"))?;

    match chosen {
        Some(path) => {
            let path_str = path.to_string();
            std::fs::write(&path_str, content).map_err(|e| {
                error!("Failed to write markdown export to {}: {}", path_str, e);
                format!("Failed to write file: {e}")
            })?;
            info!("Exported meeting markdown to {}", path_str);
            Ok(Some(path_str))
        }
        None => {
            info!("User cancelled markdown export save dialog");
            Ok(None)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExportAllResult {
    pub folder: Option<String>,
    pub exported: usize,
}

/// Build one meeting's Obsidian-style markdown: YAML frontmatter + optional
/// rendered summary + the transcript. Pure, so it is unit-testable.
pub fn build_meeting_markdown(
    title: &str,
    created_at_rfc3339: &str,
    meeting_id: &str,
    transcripts: &[Transcript],
    summary_result: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_value(title)));
    out.push_str(&format!("date: {}\n", created_at_rfc3339));
    out.push_str(&format!("meeting_id: {}\n", meeting_id));
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", title));

    if let Some(raw) = summary_result {
        if let Some(value) = crate::mcp::tools::parse_summary(raw) {
            let rendered = crate::mcp::tools::render_summary(&value);
            if !rendered.trim().is_empty() {
                out.push_str("## Summary\n\n");
                out.push_str(rendered.trim());
                out.push_str("\n\n");
            }
        }
    }

    out.push_str("## Transcript\n\n");
    for seg in transcripts {
        let text = seg.transcript.trim();
        if text.is_empty() {
            continue;
        }
        match seg.speaker.as_deref().filter(|s| !s.is_empty()) {
            Some(sp) => out.push_str(&format!("**{}:** {}\n\n", sp, text)),
            None => out.push_str(&format!("{}\n\n", text)),
        }
    }
    out
}

/// YAML-safe scalar: quote when the value could be misparsed.
fn yaml_value(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.contains(':')
        || s.contains('#')
        || s.contains('"')
        || s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace);
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// `<slug>-<YYYY-MM-DD>.md` filename for a meeting.
fn export_filename(title: &str, created_at_rfc3339: &str) -> String {
    let date = created_at_rfc3339.get(0..10).unwrap_or("undated");
    let raw: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let slug: String = raw
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "meeting".to_string() } else { slug };
    format!("{}-{}.md", slug, date)
}

/// Export every meeting to a chosen folder as an individual markdown file.
#[tauri::command]
pub async fn export_all_markdown<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<ExportAllResult, String> {
    let app_clone = app.clone();
    let folder =
        tokio::task::spawn_blocking(move || app_clone.dialog().file().blocking_pick_folder())
            .await
            .map_err(|e| format!("Folder dialog task failed: {e}"))?;
    let folder = match folder {
        Some(f) => f.to_string(),
        None => {
            info!("User cancelled bulk export folder picker");
            return Ok(ExportAllResult {
                folder: None,
                exported: 0,
            });
        }
    };

    let pool = state.db_manager.pool();
    let meetings = MeetingsRepository::get_meetings(pool)
        .await
        .map_err(|e| format!("Failed to list meetings: {e}"))?;

    let mut exported = 0usize;
    for m in &meetings {
        let created = m.created_at.0.to_rfc3339();
        let (transcripts, _total) =
            MeetingsRepository::get_meeting_transcripts_paginated(pool, &m.id, 1_000_000, 0)
                .await
                .map_err(|e| format!("Failed to load transcripts for {}: {e}", m.id))?;
        let summary = SummaryProcessesRepository::get_summary_data(pool, &m.id)
            .await
            .ok()
            .flatten();
        let summary_result = summary.as_ref().and_then(|s| s.result.as_deref());

        let md = build_meeting_markdown(&m.title, &created, &m.id, &transcripts, summary_result);
        let path = std::path::Path::new(&folder).join(export_filename(&m.title, &created));
        if let Err(e) = std::fs::write(&path, md) {
            error!("Failed to write {}: {}", path.display(), e);
            continue;
        }
        exported += 1;
    }

    info!("Bulk export wrote {} meeting file(s) to {}", exported, folder);
    Ok(ExportAllResult {
        folder: Some(folder),
        exported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_filename_slugifies_title_and_date() {
        assert_eq!(
            export_filename("Team Standup!", "2026-07-13T10:00:00+00:00"),
            "team-standup-2026-07-13.md"
        );
        assert_eq!(export_filename("   ", "2026-01-02T00:00:00Z"), "meeting-2026-01-02.md");
        assert_eq!(
            export_filename("Reunión: Q3", "2026-12-31T23:59:59Z"),
            "reunión-q3-2026-12-31.md"
        );
    }

    #[test]
    fn build_markdown_has_frontmatter_and_transcript() {
        let md = build_meeting_markdown(
            "Weekly: Sync",
            "2026-07-13T10:00:00+00:00",
            "meeting-1",
            &[],
            None,
        );
        assert!(md.starts_with("---\n"));
        assert!(md.contains("title: \"Weekly: Sync\""), "a title with ':' is quoted");
        assert!(md.contains("date: 2026-07-13T10:00:00+00:00"));
        assert!(md.contains("meeting_id: meeting-1"));
        assert!(md.contains("## Transcript"));
    }
}
