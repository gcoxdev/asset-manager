-- Schema v10: care and service history.
--
-- A watch serviced, a guitar set up, an appraisal renewed, a warranty that
-- runs out: dated entries with who did it, what it cost and, optionally,
-- the invoice already attached to the asset. An entry can say when the next
-- one is due; the overview lists what is due soon. Care costs are not part
-- of the acquisition cost — servicing a watch does not change what it cost
-- to buy.
CREATE TABLE care_events (
    care_id          TEXT    PRIMARY KEY,
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    kind             TEXT    NOT NULL
                             CHECK (kind IN ('service', 'repair', 'inspection', 'cleaning',
                                             'appraisal', 'battery', 'warranty', 'other')),
    performed_on     TEXT,
    provider         TEXT,
    cost_minor       INTEGER,
    currency         TEXT,
    note             TEXT    NOT NULL DEFAULT '',
    object_id        TEXT,
    next_due         TEXT,
    recorded_at      TEXT    NOT NULL,
    CHECK ((cost_minor IS NULL) = (currency IS NULL)),
    CHECK (performed_on IS NOT NULL OR next_due IS NOT NULL)
) STRICT;

CREATE INDEX care_events_asset ON care_events (asset_id, performed_on);
