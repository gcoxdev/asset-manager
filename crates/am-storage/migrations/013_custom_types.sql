-- Schema v13: types the owner defines.
--
-- No list of built-in types covers everyone — model trains, quilts, rare
-- seeds. A custom type is an ordinary asset type (so assets reference it
-- the same way) plus a definition of its fields: a label, a kind (text,
-- number, date, yes/no, or one of a list) and whether it is required.
-- Definitions are data, never code, and are validated in the backend on
-- every save exactly like the built-in collectible schemas.
CREATE TABLE custom_types (
    type_id          TEXT    PRIMARY KEY REFERENCES asset_types(type_id) ON DELETE CASCADE,
    blurb            TEXT    NOT NULL DEFAULT '',
    fields           TEXT    NOT NULL CHECK (json_valid(fields)),
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL
) STRICT;
