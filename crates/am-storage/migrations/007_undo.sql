-- Schema v7: mistakes can be undone.
--
-- 1. Deleting moves an asset to the trash. It drops out of every list,
--    total, search, report and export, but keeps its history, photos and
--    documents, so it can be restored. Items are purged for good after 30
--    days in the trash, or when the trash is emptied. Until then the data is
--    still in the (encrypted) vault — the app says so rather than calling
--    deletion immediate.
ALTER TABLE assets ADD COLUMN deleted_at TEXT;

-- 2. A mistyped valuation is voided, not deleted: it stops counting toward
--    any figure, and stays in the history with the reason.
ALTER TABLE valuations ADD COLUMN voided_at TEXT;
ALTER TABLE valuations ADD COLUMN void_reason TEXT NOT NULL DEFAULT '';

-- 3. Each edit keeps the record as it was before, so an edit can be undone
--    by restoring an earlier version (which is itself an edit, and undoable).
CREATE TABLE asset_revisions (
    revision_id      TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    snapshot         TEXT    NOT NULL CHECK (json_valid(snapshot)),
    recorded_at      TEXT    NOT NULL
) STRICT;

CREATE INDEX asset_revisions_asset ON asset_revisions (asset_id, recorded_at);
