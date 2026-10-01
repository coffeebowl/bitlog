-- The state of each indexed file when it was last read, so that only changed
-- files are read again. `path` is relative to the vault, `modified` is in
-- nanoseconds since 1970, `hash` is the core's content hash. Files without a
-- row here, as all of them after this migration, are read again.

CREATE TABLE files (
    path TEXT PRIMARY KEY,
    modified INTEGER NOT NULL,
    size INTEGER NOT NULL,
    hash INTEGER NOT NULL
) STRICT;
