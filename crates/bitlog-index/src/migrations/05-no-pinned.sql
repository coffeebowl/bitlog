-- Projects are no longer pinned; the settings give their order instead,
-- which the index has no use for.

ALTER TABLE projects DROP COLUMN pinned;
