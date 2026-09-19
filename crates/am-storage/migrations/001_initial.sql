-- Asset Manager schema v1.
--
-- Conventions that are NOT negotiable, because retrofitting them means
-- migrating live encrypted user data:
--
--   * Money is (amount_minor INTEGER, currency TEXT). Never a float, and
--     every monetary column carries its own currency.
--   * Quantities, weights, purities and unit quotes are exact decimal stored
--     as TEXT, with a companion REAL sort key used ONLY for ordering.
--     TEXT sorts lexicographically, so "9" > "10" without it.
--   * Timestamps are ISO-8601 UTC strings.

-- Pairs this database with its vault.header. A mismatch means a partial or
-- mixed restore; see header::check_pairing.
CREATE TABLE vault_meta (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    vault_id         TEXT    NOT NULL,
    key_epoch        INTEGER NOT NULL,
    schema_version   INTEGER NOT NULL,
    created_at       TEXT    NOT NULL
) STRICT;

-- Immutable encrypted blobs. No asset reference: one object may be shared by
-- several assets, which is why deletion is refcounted rather than cascading.
CREATE TABLE objects (
    object_id        TEXT    PRIMARY KEY,          -- random 128-bit, hex
    plaintext_sha256 TEXT    NOT NULL,             -- dedup index; never on disk
    ciphertext_bytes INTEGER NOT NULL,
    media_type       TEXT    NOT NULL,
    width            INTEGER,
    height           INTEGER,
    -- GC state machine. A crash between "drop the last association" and
    -- "unlink the file" must leave a sweepable record, not a silent orphan
    -- or a dangling reference.
    refcount         INTEGER NOT NULL DEFAULT 0 CHECK (refcount >= 0),
    gc_state         TEXT    NOT NULL DEFAULT 'live'
                             CHECK (gc_state IN ('live', 'pending_delete', 'deleted')),
    created_at       TEXT    NOT NULL
) STRICT;

CREATE UNIQUE INDEX objects_dedup ON objects (plaintext_sha256);
CREATE INDEX objects_gc ON objects (gc_state) WHERE gc_state != 'live';

CREATE TABLE asset_types (
    type_id          TEXT    PRIMARY KEY,
    archetype        TEXT    NOT NULL
                             CHECK (archetype IN ('weight', 'unit', 'unique', 'appraised', 'contractual')),
    display_name     TEXT    NOT NULL,
    schema_version   INTEGER NOT NULL DEFAULT 1
) STRICT;

CREATE TABLE assets (
    asset_id         TEXT    PRIMARY KEY,
    type_id          TEXT    NOT NULL REFERENCES asset_types(type_id),
    name             TEXT    NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'active'
                             CHECK (status IN ('active', 'sold', 'lost', 'retired')),

    -- Cache derived from asset_events, so the two can never disagree.
    -- Rebuildable; see am-storage::events.
    quantity         TEXT    NOT NULL DEFAULT '1',
    quantity_sort    REAL    NOT NULL DEFAULT 1.0,   -- ordering ONLY, never arithmetic
    quantity_unit    TEXT    NOT NULL DEFAULT 'item',

    acquired_date    TEXT,
    acquired_amount_minor INTEGER,
    acquired_currency     TEXT,
    acquired_from    TEXT,

    storage_location TEXT,
    notes            TEXT    NOT NULL DEFAULT '',

    -- Cache derived from valuations.
    current_amount_minor  INTEGER,
    current_currency      TEXT,
    value_source     TEXT,
    value_asof       TEXT,

    insured_amount_minor  INTEGER,
    insured_currency      TEXT,
    sold_date        TEXT,
    sold_amount_minor     INTEGER,
    sold_currency         TEXT,

    -- Per-type fields, validated against the type's versioned schema on every
    -- write path in the backend, not just the UI.
    attrs            TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(attrs)),
    schema_version   INTEGER NOT NULL DEFAULT 1,

    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,

    -- Money without its currency is a bug waiting to happen.
    CHECK ((acquired_amount_minor IS NULL) = (acquired_currency IS NULL)),
    CHECK ((current_amount_minor  IS NULL) = (current_currency  IS NULL)),
    CHECK ((insured_amount_minor  IS NULL) = (insured_currency  IS NULL)),
    CHECK ((sold_amount_minor     IS NULL) = (sold_currency     IS NULL))
) STRICT;

CREATE INDEX assets_type   ON assets (type_id);
CREATE INDEX assets_status ON assets (status);
CREATE INDEX assets_qty    ON assets (quantity_sort);

-- Effective-dated ownership changes.
--
-- Created in Phase 1a deliberately, though Phase 1a only ever writes one
-- 'acquire' row per asset. Phase 2 makes this the source of truth for
-- historical portfolio value: today's quantity applied to past quotes
-- silently rewrites history. Adding this later is a migration over live
-- encrypted data.
--
-- This is NOT tax-lot tracking: no FIFO/LIFO, no realized gains.
CREATE TABLE asset_events (
    event_id         TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    event_type       TEXT    NOT NULL
                             CHECK (event_type IN ('acquire', 'add', 'remove', 'dispose', 'correct')),
    effective_date   TEXT    NOT NULL,          -- when it happened
    quantity_delta   TEXT    NOT NULL,          -- signed exact decimal
    amount_minor     INTEGER,                   -- consideration, if any
    currency         TEXT,
    note             TEXT    NOT NULL DEFAULT '',
    recorded_at      TEXT    NOT NULL,          -- when we learned of it
    CHECK ((amount_minor IS NULL) = (currency IS NULL))
) STRICT;

CREATE INDEX asset_events_asset ON asset_events (asset_id, effective_date);

CREATE TABLE asset_media (
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    object_id        TEXT    NOT NULL REFERENCES objects(object_id),
    sort_order       INTEGER NOT NULL DEFAULT 0,
    is_primary       INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    captured_at      TEXT,
    created_at       TEXT    NOT NULL,
    PRIMARY KEY (asset_id, object_id)
) STRICT;

CREATE INDEX asset_media_object ON asset_media (object_id);
CREATE UNIQUE INDEX asset_media_primary ON asset_media (asset_id) WHERE is_primary = 1;

-- Thumbnails get their own opaque object IDs, so the filename leaks neither
-- the original it derives from nor which sizes exist.
CREATE TABLE media_variants (
    object_id        TEXT    NOT NULL REFERENCES objects(object_id) ON DELETE CASCADE,
    variant          TEXT    NOT NULL,          -- e.g. 'thumb:256'
    variant_object_id TEXT   NOT NULL,
    created_at       TEXT    NOT NULL,
    PRIMARY KEY (object_id, variant)
) STRICT;

-- Market quotes, separate from asset valuations: one instrument is quoted
-- once and reused across every holding that references it.
CREATE TABLE quotes (
    quote_id         TEXT    PRIMARY KEY,
    instrument_id    TEXT    NOT NULL,          -- e.g. 'metal:XAU', 'coingecko:bitcoin'
    unit_quote       TEXT    NOT NULL,          -- exact decimal, NOT minor units
    currency         TEXT    NOT NULL,
    quote_unit       TEXT    NOT NULL,          -- 'troy_oz', 'coin', ...
    source           TEXT    NOT NULL,
    match_quality    TEXT    NOT NULL DEFAULT 'exact'
                             CHECK (match_quality IN ('exact', 'approximate', 'manual')),
    source_asof      TEXT    NOT NULL,          -- when the source priced it
    fetched_at       TEXT    NOT NULL           -- when we retrieved it
) STRICT;

CREATE INDEX quotes_instrument ON quotes (instrument_id, source_asof DESC);

-- What a holding was worth, and the inputs used. Keeping quantity here is
-- what makes historical charts correct after a quantity change.
CREATE TABLE valuations (
    valuation_id     TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    quote_id         TEXT    REFERENCES quotes(quote_id),
    amount_minor     INTEGER NOT NULL,
    currency         TEXT    NOT NULL,
    quantity_at_time TEXT    NOT NULL,
    basis            TEXT    NOT NULL DEFAULT 'estimated_resale'
                             CHECK (basis IN ('melt', 'replacement', 'insured', 'estimated_resale')),
    provenance       TEXT    NOT NULL
                             CHECK (provenance IN ('manual', 'api', 'appraisal')),
    inputs           TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(inputs)),
    asof             TEXT    NOT NULL,
    recorded_at      TEXT    NOT NULL
) STRICT;

CREATE INDEX valuations_asset ON valuations (asset_id, asof DESC);

-- Persisted API quota accounting: metals.dev allows 100 requests/month, so
-- the budget has to survive restarts to mean anything.
CREATE TABLE provider_quota (
    provider         TEXT    NOT NULL,
    period           TEXT    NOT NULL,          -- 'YYYY-MM'
    used             INTEGER NOT NULL DEFAULT 0,
    limit_total      INTEGER NOT NULL,
    PRIMARY KEY (provider, period)
) STRICT;

CREATE VIRTUAL TABLE assets_fts USING fts5(
    name,
    notes,
    storage_location,
    content='assets',
    content_rowid='rowid'
);

CREATE TRIGGER assets_fts_insert AFTER INSERT ON assets BEGIN
    INSERT INTO assets_fts (rowid, name, notes, storage_location)
    VALUES (new.rowid, new.name, new.notes, coalesce(new.storage_location, ''));
END;

CREATE TRIGGER assets_fts_delete AFTER DELETE ON assets BEGIN
    INSERT INTO assets_fts (assets_fts, rowid, name, notes, storage_location)
    VALUES ('delete', old.rowid, old.name, old.notes, coalesce(old.storage_location, ''));
END;

CREATE TRIGGER assets_fts_update AFTER UPDATE ON assets BEGIN
    INSERT INTO assets_fts (assets_fts, rowid, name, notes, storage_location)
    VALUES ('delete', old.rowid, old.name, old.notes, coalesce(old.storage_location, ''));
    INSERT INTO assets_fts (rowid, name, notes, storage_location)
    VALUES (new.rowid, new.name, new.notes, coalesce(new.storage_location, ''));
END;

INSERT INTO asset_types (type_id, archetype, display_name) VALUES
    ('generic',          'unique',    'Generic Item'),
    ('gold_bullion',     'weight',    'Gold Bullion'),
    ('silver_bullion',   'weight',    'Silver Bullion'),
    ('platinum_bullion', 'weight',    'Platinum Bullion'),
    ('junk_silver',      'weight',    'Junk / Constitutional Silver'),
    ('sovereign_coin',   'weight',    'Sovereign Coin');
