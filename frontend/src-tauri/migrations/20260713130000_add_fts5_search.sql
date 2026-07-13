-- Unified full-text search index (FTS5), shared by app search, MCP search, and
-- find-in-transcript. Content-owning table so snippet()/rank work; diacritic-
-- insensitive (unicode61 remove_diacritics 2) so "reunion" matches "reunión".
--
-- One index over five content sources: transcript segments, legacy chunk blobs,
-- summaries, meeting notes, and chat messages. Each row is keyed by (source,
-- source_rowid) — the source table's implicit rowid — so triggers can target the
-- exact index row on UPDATE/DELETE. meeting_id/source/source_rowid are UNINDEXED
-- (stored, not tokenized); only `content` is searchable.
--
-- Trashed (soft-deleted) meetings are NOT removed from the index; search queries
-- JOIN meetings and filter `deleted_at IS NULL`, so restore brings matches back
-- for free.
CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
    meeting_id UNINDEXED,
    source UNINDEXED,
    source_rowid UNINDEXED,
    content,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- ---------------------------------------------------------------------------
-- Sync triggers. INSERT/UPDATE use INSERT..SELECT..WHERE so NULL/empty content
-- is skipped without gating the DELETE half (a field cleared to NULL must still
-- drop its stale index row). DELETE removes by (source, source_rowid).
-- ---------------------------------------------------------------------------

-- transcripts.transcript
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_ai AFTER INSERT ON transcripts BEGIN
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'transcript', NEW.rowid, NEW.transcript
    WHERE NEW.transcript IS NOT NULL AND NEW.transcript <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_au AFTER UPDATE OF transcript ON transcripts BEGIN
    DELETE FROM search_index WHERE source = 'transcript' AND source_rowid = OLD.rowid;
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'transcript', NEW.rowid, NEW.transcript
    WHERE NEW.transcript IS NOT NULL AND NEW.transcript <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_ad AFTER DELETE ON transcripts BEGIN
    DELETE FROM search_index WHERE source = 'transcript' AND source_rowid = OLD.rowid;
END;

-- transcript_chunks.transcript_text (legacy per-meeting blob)
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_ai AFTER INSERT ON transcript_chunks BEGIN
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'chunk', NEW.rowid, NEW.transcript_text
    WHERE NEW.transcript_text IS NOT NULL AND NEW.transcript_text <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_au AFTER UPDATE OF transcript_text ON transcript_chunks BEGIN
    DELETE FROM search_index WHERE source = 'chunk' AND source_rowid = OLD.rowid;
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'chunk', NEW.rowid, NEW.transcript_text
    WHERE NEW.transcript_text IS NOT NULL AND NEW.transcript_text <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_ad AFTER DELETE ON transcript_chunks BEGIN
    DELETE FROM search_index WHERE source = 'chunk' AND source_rowid = OLD.rowid;
END;

-- summary_processes.result (raw summary JSON — searchable content words)
CREATE TRIGGER IF NOT EXISTS trg_si_summary_ai AFTER INSERT ON summary_processes BEGIN
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'summary', NEW.rowid, NEW.result
    WHERE NEW.result IS NOT NULL AND NEW.result <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_summary_au AFTER UPDATE OF result ON summary_processes BEGIN
    DELETE FROM search_index WHERE source = 'summary' AND source_rowid = OLD.rowid;
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'summary', NEW.rowid, NEW.result
    WHERE NEW.result IS NOT NULL AND NEW.result <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_summary_ad AFTER DELETE ON summary_processes BEGIN
    DELETE FROM search_index WHERE source = 'summary' AND source_rowid = OLD.rowid;
END;

-- meeting_notes.notes_markdown
CREATE TRIGGER IF NOT EXISTS trg_si_notes_ai AFTER INSERT ON meeting_notes BEGIN
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'note', NEW.rowid, NEW.notes_markdown
    WHERE NEW.notes_markdown IS NOT NULL AND NEW.notes_markdown <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_notes_au AFTER UPDATE OF notes_markdown ON meeting_notes BEGIN
    DELETE FROM search_index WHERE source = 'note' AND source_rowid = OLD.rowid;
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'note', NEW.rowid, NEW.notes_markdown
    WHERE NEW.notes_markdown IS NOT NULL AND NEW.notes_markdown <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_notes_ad AFTER DELETE ON meeting_notes BEGIN
    DELETE FROM search_index WHERE source = 'note' AND source_rowid = OLD.rowid;
END;

-- chat_messages.content
CREATE TRIGGER IF NOT EXISTS trg_si_chat_ai AFTER INSERT ON chat_messages BEGIN
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'chat', NEW.rowid, NEW.content
    WHERE NEW.content IS NOT NULL AND NEW.content <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chat_au AFTER UPDATE OF content ON chat_messages BEGIN
    DELETE FROM search_index WHERE source = 'chat' AND source_rowid = OLD.rowid;
    INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT NEW.meeting_id, 'chat', NEW.rowid, NEW.content
    WHERE NEW.content IS NOT NULL AND NEW.content <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chat_ad AFTER DELETE ON chat_messages BEGIN
    DELETE FROM search_index WHERE source = 'chat' AND source_rowid = OLD.rowid;
END;

-- Safety net: children of a hard-deleted meeting are removed via FK ON DELETE
-- CASCADE (meeting_notes, chat_messages) which does NOT fire their per-row DELETE
-- triggers under SQLite's default recursive_triggers=OFF. This catches every
-- index row for the meeting regardless of how the children went away.
CREATE TRIGGER IF NOT EXISTS trg_si_meetings_ad AFTER DELETE ON meetings BEGIN
    DELETE FROM search_index WHERE meeting_id = OLD.id;
END;

-- ---------------------------------------------------------------------------
-- Backfill existing content.
-- ---------------------------------------------------------------------------
INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT meeting_id, 'transcript', rowid, transcript
    FROM transcripts WHERE transcript IS NOT NULL AND transcript <> '';
INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT meeting_id, 'chunk', rowid, transcript_text
    FROM transcript_chunks WHERE transcript_text IS NOT NULL AND transcript_text <> '';
INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT meeting_id, 'summary', rowid, result
    FROM summary_processes WHERE result IS NOT NULL AND result <> '';
INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT meeting_id, 'note', rowid, notes_markdown
    FROM meeting_notes WHERE notes_markdown IS NOT NULL AND notes_markdown <> '';
INSERT INTO search_index(meeting_id, source, source_rowid, content)
    SELECT meeting_id, 'chat', rowid, content
    FROM chat_messages WHERE content IS NOT NULL AND content <> '';
