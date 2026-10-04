-- Schema v11: who has it.
--
-- Lent to a friend, consigned to a dealer, at the repair shop, in outside
-- storage, in the post — and back again. Each handoff is dated, with who
-- took it, when it is due back, a reference (a consignment or tracking
-- number) and optionally the receipt attached to the asset. Ownership and
-- value are unaffected: a consigned watch is still yours until it sells.
-- Contact details are personal data about someone else; they are kept in
-- the encrypted vault and never included in exports or reports.
CREATE TABLE custody_events (
    custody_id       TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    kind             TEXT    NOT NULL
                             CHECK (kind IN ('lent', 'consigned', 'repair', 'storage', 'shipped', 'returned')),
    party            TEXT,
    contact          TEXT,
    date             TEXT    NOT NULL,
    due_back         TEXT,
    reference        TEXT,
    note             TEXT    NOT NULL DEFAULT '',
    object_id        TEXT,
    recorded_at      TEXT    NOT NULL
) STRICT;

CREATE INDEX custody_events_asset ON custody_events (asset_id, date);
