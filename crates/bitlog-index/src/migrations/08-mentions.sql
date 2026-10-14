-- Mentions of contacts like `@kim`, one row per mention, whether the
-- contact exists or not. A mention lies in a note or a day file, as a link
-- does (see `links`).

CREATE TABLE mentions (
    note_project TEXT,
    note_name TEXT,
    date TEXT REFERENCES days (date) ON DELETE CASCADE,
    block TEXT,
    contact TEXT NOT NULL,
    FOREIGN KEY (note_project, note_name) REFERENCES notes (project, name) ON DELETE CASCADE,
    CHECK ((note_name IS NULL) <> (date IS NULL))
) STRICT;

CREATE INDEX mentions_by_contact ON mentions (contact);
CREATE INDEX mentions_by_note ON mentions (note_project, note_name);
CREATE INDEX mentions_by_date ON mentions (date);

-- Mentions are found while reading files: read all of them again.
DELETE FROM files;
