-- Schema v3: a catalog people can actually live in.
--
-- Four changes, each correcting something the v1/v2 schema got wrong or
-- left out.

-- 1. Effective dates are calendar dates.
--
-- Earlier builds wrote the acquire event's effective_date as a full RFC 3339
-- timestamp ("2026-09-22T14:03:11Z") while every reader compares against a
-- plain date ("2026-09-22"). Text comparison puts the timestamp *after* the
-- date, so an asset created today counted as not yet held today: it was left
-- out of today's total and valued at quantity zero. Truncating is exact — the
-- date part is the date the user meant.
UPDATE asset_events
SET effective_date = substr(effective_date, 1, 10)
WHERE length(effective_date) > 10;

-- 2. Categories and the types the forms actually create.
--
-- Collectibles were stored as 'generic' because their types had no row here,
-- which lost the one fact the form had asked for first. The category groups
-- types for allocation views; it is presentation, not valuation.
ALTER TABLE asset_types ADD COLUMN category TEXT NOT NULL DEFAULT 'other';

UPDATE asset_types SET category = 'metals'
WHERE type_id IN ('gold_bullion', 'silver_bullion', 'platinum_bullion', 'junk_silver', 'sovereign_coin');

INSERT INTO asset_types (type_id, archetype, display_name, category) VALUES
    ('palladium_bullion', 'weight',      'Palladium Bullion',  'metals'),
    ('crypto',            'unit',        'Cryptocurrency',     'crypto'),
    ('comic',             'unique',      'Comic Book',         'collectibles'),
    ('trading_card',      'unique',      'Trading Card',       'collectibles'),
    ('tcg_card',          'unique',      'TCG Card',           'collectibles'),
    ('numismatic_coin',   'unique',      'Numismatic Coin',    'collectibles'),
    ('memorabilia',       'unique',      'Memorabilia',        'collectibles'),
    ('sealed_product',    'unique',      'Sealed Product',     'collectibles'),
    ('video_game',        'unique',      'Video Game',         'collectibles'),
    ('vinyl',             'unique',      'Vinyl Record',       'collectibles'),
    ('art',               'unique',      'Art & Prints',       'valuables'),
    ('watch',             'unique',      'Watch',              'valuables'),
    ('jewelry',           'unique',      'Jewelry',            'valuables'),
    ('instrument',        'appraised',   'Musical Instrument', 'valuables'),
    ('wine',              'appraised',   'Wine & Spirits',     'valuables'),
    ('cash',              'contractual', 'Cash & Accounts',    'cash');

-- Recover the type of collectibles saved as 'generic', from the fields each
-- type requires. Order matters only where requirements could overlap, and
-- they do not: each type has a distinguishing required field.
UPDATE assets SET type_id = 'comic'
WHERE type_id = 'generic'
  AND json_extract(attrs, '$.title') IS NOT NULL
  AND json_extract(attrs, '$.issue') IS NOT NULL;

UPDATE assets SET type_id = 'trading_card'
WHERE type_id = 'generic'
  AND json_extract(attrs, '$.player_or_character') IS NOT NULL
  AND json_extract(attrs, '$.set') IS NOT NULL;

UPDATE assets SET type_id = 'tcg_card'
WHERE type_id = 'generic'
  AND json_extract(attrs, '$.name') IS NOT NULL
  AND json_extract(attrs, '$.set') IS NOT NULL;

UPDATE assets SET type_id = 'numismatic_coin'
WHERE type_id = 'generic'
  AND json_extract(attrs, '$.denomination') IS NOT NULL
  AND json_extract(attrs, '$.year') IS NOT NULL;

-- 3. How each asset is priced, and when to look again.
--
-- 'market' assets are revalued from stored quotes (spot metal, coin prices);
-- 'manual' assets are only ever valued by hand. Entering a price by hand
-- switches an asset to 'manual', which is the precedence rule the plan asks
-- for: a refresh can never overwrite a hand-entered valuation, because an
-- asset with one no longer follows the market until the owner says so.
ALTER TABLE assets ADD COLUMN pricing TEXT NOT NULL DEFAULT 'manual'
    CHECK (pricing IN ('manual', 'market'));

-- Optional recheck reminder. Null means never nag.
ALTER TABLE assets ADD COLUMN review_every_days INTEGER
    CHECK (review_every_days IS NULL OR review_every_days > 0);

-- 4. Search covers type-specific fields.
--
-- Cert numbers, grades and set names live in attrs; a search for a cert
-- number found nothing. Rebuilt rather than altered: FTS5 tables cannot gain
-- columns in place.
DROP TRIGGER assets_fts_insert;
DROP TRIGGER assets_fts_delete;
DROP TRIGGER assets_fts_update;
DROP TABLE assets_fts;

CREATE VIRTUAL TABLE assets_fts USING fts5(
    name,
    notes,
    storage_location,
    attrs,
    content='assets',
    content_rowid='rowid'
);

CREATE TRIGGER assets_fts_insert AFTER INSERT ON assets BEGIN
    INSERT INTO assets_fts (rowid, name, notes, storage_location, attrs)
    VALUES (new.rowid, new.name, new.notes, coalesce(new.storage_location, ''), new.attrs);
END;

CREATE TRIGGER assets_fts_delete AFTER DELETE ON assets BEGIN
    INSERT INTO assets_fts (assets_fts, rowid, name, notes, storage_location, attrs)
    VALUES ('delete', old.rowid, old.name, old.notes, coalesce(old.storage_location, ''), old.attrs);
END;

CREATE TRIGGER assets_fts_update AFTER UPDATE ON assets BEGIN
    INSERT INTO assets_fts (assets_fts, rowid, name, notes, storage_location, attrs)
    VALUES ('delete', old.rowid, old.name, old.notes, coalesce(old.storage_location, ''), old.attrs);
    INSERT INTO assets_fts (rowid, name, notes, storage_location, attrs)
    VALUES (new.rowid, new.name, new.notes, coalesce(new.storage_location, ''), new.attrs);
END;

INSERT INTO assets_fts (assets_fts) VALUES ('rebuild');
