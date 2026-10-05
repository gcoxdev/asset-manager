-- Schema v17: splitting a holding, and sets.
--
-- 1. A "split" event: part of a holding becomes an item of its own (three
--    coins from a roll, one card from a lot). The units leave this asset
--    without being sold — so it is not a "remove" — and arrive as the new
--    asset's acquisition on the same day. SQLite cannot widen a CHECK in
--    place, so the table is rebuilt with every row copied in order.
CREATE TABLE asset_events_v17 (
    event_id         TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    event_type       TEXT    NOT NULL
                             CHECK (event_type IN ('acquire', 'add', 'remove', 'dispose', 'correct', 'split')),
    effective_date   TEXT    NOT NULL,
    quantity_delta   TEXT    NOT NULL,
    amount_minor     INTEGER,
    currency         TEXT,
    note             TEXT    NOT NULL DEFAULT '',
    recorded_at      TEXT    NOT NULL,
    CHECK ((amount_minor IS NULL) = (currency IS NULL))
) STRICT;

INSERT INTO asset_events_v17
    (event_id, asset_id, event_type, effective_date, quantity_delta, amount_minor, currency, note, recorded_at)
SELECT event_id, asset_id, event_type, effective_date, quantity_delta, amount_minor, currency, note, recorded_at
FROM asset_events ORDER BY rowid;

DROP TABLE asset_events;
ALTER TABLE asset_events_v17 RENAME TO asset_events;
CREATE INDEX asset_events_asset ON asset_events (asset_id, effective_date);

-- 2. Sets: a coin set, a card checklist, a tool kit. A set has no value of
--    its own — it is the sum of its members — so nothing is counted twice.
CREATE TABLE sets (
    set_id           TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    target_count     INTEGER CHECK (target_count IS NULL OR target_count > 0),
    notes            TEXT    NOT NULL DEFAULT '',
    created_at       TEXT    NOT NULL
) STRICT;

CREATE TABLE set_members (
    set_id           TEXT    NOT NULL REFERENCES sets(set_id) ON DELETE CASCADE,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    added_at         TEXT    NOT NULL,
    PRIMARY KEY (set_id, asset_id)
) STRICT;

CREATE INDEX set_members_asset ON set_members (asset_id);
