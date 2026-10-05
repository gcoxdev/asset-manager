-- Schema v15: things wanted, kept apart from things owned.
--
-- A wish is not an asset: it has no quantity held, no value, no history,
-- and it never enters a total. It lives in its own table so no query over
-- assets can count it by mistake. When it is bought, the asset created for
-- it is recorded here and the wish is kept, marked as got.
CREATE TABLE wishes (
    wish_id          TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 300),
    type_id          TEXT    NOT NULL REFERENCES asset_types(type_id),
    target_minor     INTEGER,
    currency         TEXT,
    quantity         TEXT    NOT NULL DEFAULT '1',
    priority         TEXT    NOT NULL DEFAULT 'normal' CHECK (priority IN ('low', 'normal', 'high')),
    notes            TEXT    NOT NULL DEFAULT '',
    acquired_asset_id TEXT,
    acquired_at      TEXT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    CHECK ((target_minor IS NULL) = (currency IS NULL))
) STRICT;
