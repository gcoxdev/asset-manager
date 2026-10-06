-- Keep expected membership and observations independently of live assets.
CREATE TABLE inventory_check_items (
    check_id TEXT NOT NULL REFERENCES inventory_checks(check_id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL,
    name TEXT NOT NULL,
    type_label TEXT NOT NULL,
    storage_location TEXT,
    quantity TEXT NOT NULL,
    quantity_unit TEXT NOT NULL,
    away TEXT,
    result TEXT,
    counted TEXT,
    baseline TEXT,
    reconciled_at TEXT,
    PRIMARY KEY (check_id, asset_id)
) STRICT;
-- Older checks have no recoverable baseline. Preserve their current review,
-- but NULL baseline prevents applying their observations to today's holding.
INSERT INTO inventory_check_items
    (check_id, asset_id, name, type_label, storage_location, quantity, quantity_unit, away, result, counted)
SELECT c.check_id, a.asset_id, a.name, t.display_name, a.storage_location, a.quantity, a.quantity_unit,
       NULL, m.result, m.counted
FROM inventory_checks c JOIN assets a JOIN asset_types t ON t.type_id = a.type_id
LEFT JOIN inventory_marks m ON m.check_id = c.check_id AND m.asset_id = a.asset_id
WHERE (a.deleted_at IS NULL AND a.status = 'active'
   AND (c.scope_location IS NULL OR a.storage_location = c.scope_location
     OR substr(a.storage_location, 1, length(c.scope_location) + 3) = c.scope_location || ' / '))
   OR m.asset_id IS NOT NULL;
