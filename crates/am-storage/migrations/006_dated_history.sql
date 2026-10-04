-- Schema v6: lifecycle and cost history with dates.
--
-- 1. Lost, retired and recovered are dated events.
--
-- Status used to be a single field. Historical totals included only assets
-- whose *current* status was active or sold, so marking a watch lost in
-- October erased it from March's total too — and a claim for that watch
-- needs exactly its pre-loss value. Now each change is an event with an
-- effective date; an asset counts on any date it was held and not then lost
-- or retired. The cached `assets.status` follows the latest event.
CREATE TABLE status_events (
    event_id         TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    status           TEXT    NOT NULL CHECK (status IN ('active', 'lost', 'retired')),
    effective_date   TEXT    NOT NULL,
    note             TEXT    NOT NULL DEFAULT '',
    recorded_at      TEXT    NOT NULL
) STRICT;

CREATE INDEX status_events_asset ON status_events (asset_id, effective_date);

-- Existing lost and retired items had no date. The last edit is the best
-- available guess, and errs toward keeping history: the item counts until
-- then rather than vanishing from every past total.
INSERT INTO status_events (event_id, asset_id, status, effective_date, note, recorded_at)
SELECT lower(hex(randomblob(16))), asset_id, status, substr(updated_at, 1, 10),
       'Marked before status changes were dated; dated from the last edit.', updated_at
FROM assets
WHERE status IN ('lost', 'retired');

-- 2. Cost is replayed in date order, from statements of what it was.
--
-- The position cost used to be adjusted as each change was *recorded*, so a
-- sale entered late was averaged against purchases made after it, and the
-- result depended on typing order. It is now rebuilt from the event log in
-- effective-date order. A cost statement is the owner saying "what I hold
-- cost this much in total" — the acquisition price, a restated total after
-- buying more at an unknown price — and resets the replay at its date.
-- Changes entered later but dated before a statement are applied after it,
-- since the statement could not have accounted for them.
CREATE TABLE cost_statements (
    statement_id     TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    effective_date   TEXT    NOT NULL,
    amount_minor     INTEGER,
    currency         TEXT,
    -- Whether the amount covers everything then held (see cost_complete).
    covers_holding   INTEGER NOT NULL DEFAULT 1 CHECK (covers_holding IN (0, 1)),
    note             TEXT    NOT NULL DEFAULT '',
    recorded_at      TEXT    NOT NULL,
    CHECK ((amount_minor IS NULL) = (currency IS NULL))
) STRICT;

CREATE INDEX cost_statements_asset ON cost_statements (asset_id, effective_date);

-- Every existing figure is kept exactly: today's recorded cost becomes a
-- statement as of the last change, so replaying from it changes nothing.
INSERT INTO cost_statements
    (statement_id, asset_id, effective_date, amount_minor, currency, covers_holding, note, recorded_at)
SELECT lower(hex(randomblob(16))), a.asset_id,
       (SELECT max(e.effective_date) FROM asset_events e WHERE e.asset_id = a.asset_id),
       a.acquired_amount_minor, a.acquired_currency, a.cost_complete,
       'Cost as recorded before date-ordered replay.',
       strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM assets a
WHERE EXISTS (SELECT 1 FROM asset_events e WHERE e.asset_id = a.asset_id);
