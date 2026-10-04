-- Schema v9: tags.
--
-- Categories say what something is; tags say whatever else the owner needs
-- to find it by — "insured rider", "for sale", "Grandma's". Many per asset,
-- matched without regard to case. Like every other label, they are inside
-- the encrypted database: a tag can be as revealing as a note.
CREATE TABLE tags (
    tag_id           TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL UNIQUE COLLATE NOCASE
                             CHECK (length(trim(name)) BETWEEN 1 AND 60),
    created_at       TEXT    NOT NULL
) STRICT;

CREATE TABLE asset_tags (
    asset_id         TEXT    NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    tag_id           TEXT    NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
    PRIMARY KEY (asset_id, tag_id)
) STRICT;

CREATE INDEX asset_tags_tag ON asset_tags (tag_id);
