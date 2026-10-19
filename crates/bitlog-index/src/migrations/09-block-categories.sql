-- The category of a block, `work` unless the day file says otherwise: read
-- the day files again.

ALTER TABLE blocks ADD COLUMN category TEXT NOT NULL DEFAULT 'work';

DELETE FROM files WHERE path LIKE 'days/%';
