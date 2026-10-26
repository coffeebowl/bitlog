-- Blocks no longer have a category; projects have one instead.

ALTER TABLE blocks DROP COLUMN category;
