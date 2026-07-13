-- Unified full-text search index (FTS5), shared by app search, MCP search, and
-- find-in-transcript. Content-owning table so snippet()/rank work; diacritic-
-- insensitive (unicode61 remove_diacritics 2) so "reunion" matches "reunión".
--
-- One index over five content sources: transcript segments, legacy chunk blobs,
-- summaries, meeting notes, and chat messages. meeting_id/source/source_rowid are
-- UNINDEXED (stored, not tokenized); only `content` is searchable.
--
-- IMPORTANT — efficient sync: each index row's FTS rowid is a deterministic,
-- globally-unique encoding of its source row: `(source_rowid << 3) | source_tag`
-- with tags transcript=0, chunk=1, summary=2, note=3, chat=4. That lets the
-- UPDATE/DELETE triggers target the exact index row by rowid (O(log n)), instead
-- of scanning the whole index by the UNINDEXED (source, source_rowid) pair —
-- which would make a bulk `DELETE FROM transcripts WHERE meeting_id=?`
-- (retranscription / purge) O(segments × index_rows). Source rowids are small
-- (row counts), so `<< 3` never overflows.
--
-- Trashed (soft-deleted) meetings are NOT removed from the index; search queries
-- JOIN meetings and filter `deleted_at IS NULL`, so restore brings matches back.
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
-- drop its stale index row). All UPDATE/DELETE target the encoded FTS rowid.
-- ---------------------------------------------------------------------------

-- transcripts.transcript (tag 0)
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_ai AFTER INSERT ON transcripts BEGIN
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 0, NEW.meeting_id, 'transcript', NEW.rowid, NEW.transcript
    WHERE NEW.transcript IS NOT NULL AND NEW.transcript <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_au AFTER UPDATE OF transcript ON transcripts BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 0;
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 0, NEW.meeting_id, 'transcript', NEW.rowid, NEW.transcript
    WHERE NEW.transcript IS NOT NULL AND NEW.transcript <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_transcripts_ad AFTER DELETE ON transcripts BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 0;
END;

-- transcript_chunks.transcript_text (legacy per-meeting blob) (tag 1)
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_ai AFTER INSERT ON transcript_chunks BEGIN
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 1, NEW.meeting_id, 'chunk', NEW.rowid, NEW.transcript_text
    WHERE NEW.transcript_text IS NOT NULL AND NEW.transcript_text <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_au AFTER UPDATE OF transcript_text ON transcript_chunks BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 1;
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 1, NEW.meeting_id, 'chunk', NEW.rowid, NEW.transcript_text
    WHERE NEW.transcript_text IS NOT NULL AND NEW.transcript_text <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chunks_ad AFTER DELETE ON transcript_chunks BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 1;
END;

-- summary_processes.result (raw summary JSON — searchable content words) (tag 2)
CREATE TRIGGER IF NOT EXISTS trg_si_summary_ai AFTER INSERT ON summary_processes BEGIN
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 2, NEW.meeting_id, 'summary', NEW.rowid, NEW.result
    WHERE NEW.result IS NOT NULL AND NEW.result <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_summary_au AFTER UPDATE OF result ON summary_processes BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 2;
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 2, NEW.meeting_id, 'summary', NEW.rowid, NEW.result
    WHERE NEW.result IS NOT NULL AND NEW.result <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_summary_ad AFTER DELETE ON summary_processes BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 2;
END;

-- meeting_notes.notes_markdown (tag 3)
CREATE TRIGGER IF NOT EXISTS trg_si_notes_ai AFTER INSERT ON meeting_notes BEGIN
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 3, NEW.meeting_id, 'note', NEW.rowid, NEW.notes_markdown
    WHERE NEW.notes_markdown IS NOT NULL AND NEW.notes_markdown <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_notes_au AFTER UPDATE OF notes_markdown ON meeting_notes BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 3;
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 3, NEW.meeting_id, 'note', NEW.rowid, NEW.notes_markdown
    WHERE NEW.notes_markdown IS NOT NULL AND NEW.notes_markdown <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_notes_ad AFTER DELETE ON meeting_notes BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 3;
END;

-- chat_messages.content (tag 4)
CREATE TRIGGER IF NOT EXISTS trg_si_chat_ai AFTER INSERT ON chat_messages BEGIN
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 4, NEW.meeting_id, 'chat', NEW.rowid, NEW.content
    WHERE NEW.content IS NOT NULL AND NEW.content <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chat_au AFTER UPDATE OF content ON chat_messages BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 4;
    INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (NEW.rowid << 3) | 4, NEW.meeting_id, 'chat', NEW.rowid, NEW.content
    WHERE NEW.content IS NOT NULL AND NEW.content <> '';
END;
CREATE TRIGGER IF NOT EXISTS trg_si_chat_ad AFTER DELETE ON chat_messages BEGIN
    DELETE FROM search_index WHERE rowid = (OLD.rowid << 3) | 4;
END;

-- Safety net: children of a hard-deleted meeting are removed via FK ON DELETE
-- CASCADE (meeting_notes, chat_messages) which does NOT fire their per-row DELETE
-- triggers under SQLite's default recursive_triggers=OFF. This one-off scan (per
-- meeting hard-delete, a rare operation) catches any remaining index rows.
CREATE TRIGGER IF NOT EXISTS trg_si_meetings_ad AFTER DELETE ON meetings BEGIN
    DELETE FROM search_index WHERE meeting_id = OLD.id;
END;

-- ---------------------------------------------------------------------------
-- Backfill existing content (explicit encoded rowids, matching the triggers).
-- ---------------------------------------------------------------------------
INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (rowid << 3) | 0, meeting_id, 'transcript', rowid, transcript
    FROM transcripts WHERE transcript IS NOT NULL AND transcript <> '';
INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (rowid << 3) | 1, meeting_id, 'chunk', rowid, transcript_text
    FROM transcript_chunks WHERE transcript_text IS NOT NULL AND transcript_text <> '';
INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (rowid << 3) | 2, meeting_id, 'summary', rowid, result
    FROM summary_processes WHERE result IS NOT NULL AND result <> '';
INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (rowid << 3) | 3, meeting_id, 'note', rowid, notes_markdown
    FROM meeting_notes WHERE notes_markdown IS NOT NULL AND notes_markdown <> '';
INSERT INTO search_index(rowid, meeting_id, source, source_rowid, content)
    SELECT (rowid << 3) | 4, meeting_id, 'chat', rowid, content
    FROM chat_messages WHERE content IS NOT NULL AND content <> '';
