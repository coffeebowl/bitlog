-- Wiki links to notes, one row per link that points to a valid note path,
-- whether the note exists or not. A link lies in a note (`note_project`,
-- `note_name`) or in a day file (`date`), there in the text of the block
-- `block` or, without one, in the day note.

CREATE TABLE links (
    note_project TEXT,
    note_name TEXT,
    date TEXT REFERENCES days (date) ON DELETE CASCADE,
    block TEXT,
    target_project TEXT NOT NULL,
    target_name TEXT NOT NULL,
    FOREIGN KEY (note_project, note_name) REFERENCES notes (project, name) ON DELETE CASCADE,
    CHECK ((note_name IS NULL) <> (date IS NULL))
) STRICT;

CREATE INDEX links_by_target ON links (target_project, target_name);
CREATE INDEX links_by_note ON links (note_project, note_name);
CREATE INDEX links_by_date ON links (date);

-- Links are found while reading files: read all of them again.
DELETE FROM files;
