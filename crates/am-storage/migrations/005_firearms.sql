-- Schema v5: a firearms category.
--
-- Firearms are identified by serial number far more than most valuables, and
-- are among the items an insurer most often asks to have itemized. Their own
-- category keeps them out of "valuables" in allocation views and reports, and
-- gives them types whose forms ask for caliber and serial rather than stones
-- and hallmarks. Ammunition is counted in rounds, so it is a unit holding.
INSERT INTO asset_types (type_id, archetype, display_name, category) VALUES
    ('firearm',           'unique',      'Firearm',               'firearms'),
    ('ammunition',        'unit',        'Ammunition',            'firearms'),
    ('firearm_accessory', 'unique',      'Optics & Accessories',  'firearms');
