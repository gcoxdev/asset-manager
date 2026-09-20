-- Per-vault settings.
--
-- In the vault rather than a config file so a setting travels with the data
-- it governs: a vault restored onto another machine keeps its own choices,
-- and a privacy opt-in cannot be flipped on by editing a dotfile.
CREATE TABLE app_settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
) STRICT;
