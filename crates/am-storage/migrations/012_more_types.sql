-- Schema v12: more of what people own.
--
-- Most households' most valuable possessions are not collectibles: a car, a
-- home, electronics, furniture, tools. Each gets a type whose form asks for
-- what identifies it (a VIN, a parcel number, a serial) rather than a
-- catch-all "generic". Values are entered by hand; nothing here connects to
-- a bank, broker or registry.
--
-- Property and vehicles are recorded at the value of the owner's share; a
-- mortgage or loan is not an asset and is not recorded here, so totals are
-- what is owned, not net worth.
INSERT INTO asset_types (type_id, archetype, display_name, category) VALUES
    ('vehicle',       'unique',    'Vehicle',                  'vehicles'),
    ('boat',          'unique',    'Boat',                     'vehicles'),
    ('real_estate',   'appraised', 'Home or building',         'property'),
    ('land',          'appraised', 'Land',                     'property'),
    ('electronics',   'unique',    'Electronics',              'household'),
    ('appliance',     'unique',    'Appliance',                'household'),
    ('furniture',     'unique',    'Furniture',                'household'),
    ('tools',         'unique',    'Tools & equipment',        'household'),
    ('sports_gear',   'unique',    'Sports & outdoor gear',    'household'),
    ('fashion',       'unique',    'Fashion & accessories',    'valuables'),
    ('books',         'unique',    'Books',                    'collectibles'),
    ('stamp',         'unique',    'Stamp',                    'collectibles'),
    ('toy',           'unique',    'Toys & figures',           'collectibles'),
    ('security',      'unit',      'Stock, bond or fund',      'investments');
