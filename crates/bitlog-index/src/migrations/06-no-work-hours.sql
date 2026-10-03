-- Days no longer have working hours; working time is the sum of their
-- blocks without breaks.

ALTER TABLE days DROP COLUMN work_start;
ALTER TABLE days DROP COLUMN work_end;
