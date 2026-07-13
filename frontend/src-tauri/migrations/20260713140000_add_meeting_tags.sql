-- Free-text tags / workspaces for meetings (many-to-many). A meeting can carry
-- any number of tags; a tag is any non-empty trimmed string. Used by the sidebar
-- filter chips and by MCP list/search scoping. ON DELETE CASCADE removes a
-- meeting's tags when it is hard-purged (soft-delete leaves tags intact).
CREATE TABLE IF NOT EXISTS meeting_tags (
    meeting_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (meeting_id, tag),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- Look up meetings by tag (filter chips, MCP tag scoping).
CREATE INDEX IF NOT EXISTS idx_meeting_tags_tag ON meeting_tags(tag);
