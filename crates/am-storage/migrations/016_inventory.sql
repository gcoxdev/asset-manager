-- Schema v16: physical inventory checks.
--
-- Walk a room or a safe and confirm each thing is there. A check covers a
-- location (and the places inside it) or everything, records each item as
-- present, missing, or present in a different count, and is saved as it
-- goes so a long check can be finished another day. Discrepancies are
-- reviewed before anything changes: a check never marks an item lost or
-- rewrites a quantity by itself.
CREATE TABLE inventory_checks (
    check_id         TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL,
    scope_location   TEXT,
    started_at       TEXT    NOT NULL,
    finished_at      TEXT
) STRICT;

CREATE TABLE inventory_marks (
    check_id         TEXT    NOT NULL REFERENCES inventory_checks(check_id) ON DELETE CASCADE,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    result           TEXT    NOT NULL CHECK (result IN ('present', 'missing', 'count')),
    counted          TEXT,
    marked_at        TEXT    NOT NULL,
    PRIMARY KEY (check_id, asset_id)
) STRICT;

CREATE INDEX inventory_marks_asset ON inventory_marks (asset_id, marked_at);
