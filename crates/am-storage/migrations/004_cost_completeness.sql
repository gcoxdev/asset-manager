-- Schema v4: know when a recorded cost covers only part of a holding.
--
-- The position keeps one acquisition cost (decision 2: no tax lots). Buying
-- more without a price — or in a different currency from the recorded cost —
-- leaves that cost as it was, which is right: inventing zero would overstate
-- the gain. But the old cost then belongs to only some of the units, and
-- every gain computed against it treated it as the cost of all of them: buy
-- one at $100, add a second at an unknown price, and the position showed a
-- $100 "gain" with full coverage.
--
-- cost_complete = 1 means the recorded cost is for the whole holding (or no
-- cost is recorded, which is already "unknown"). 0 means part of the holding
-- arrived at an unknown cost; gain is not computed until the owner records
-- the total paid for the whole position. Zero cost — a gift — stays a known
-- cost: it is NULL, not 0, that means unknown.
ALTER TABLE assets ADD COLUMN cost_complete INTEGER NOT NULL DEFAULT 1
    CHECK (cost_complete IN (0, 1));

-- Existing holdings: any purchase-more without a price in the cost's currency
-- left the cost partial.
UPDATE assets SET cost_complete = 0
WHERE acquired_amount_minor IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM asset_events e
      WHERE e.asset_id = assets.asset_id
        AND e.event_type = 'add'
        AND (e.amount_minor IS NULL OR e.currency IS NOT assets.acquired_currency)
  );
