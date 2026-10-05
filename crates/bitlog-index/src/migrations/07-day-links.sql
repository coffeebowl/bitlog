-- A short wiki link like `[[name]]` in a block text no longer points to a
-- note of the block's project: read the day files again.

DELETE FROM files WHERE path LIKE 'daily/%';
