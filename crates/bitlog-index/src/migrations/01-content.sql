-- The content of the vault files, as the core reads it. Dates are
-- `YYYY-MM-DD`, times are minutes after the start of the day.

CREATE TABLE projects (
    slug TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    category TEXT NOT NULL,
    status TEXT NOT NULL,
    pinned INTEGER NOT NULL,
    created TEXT
) STRICT;

CREATE TABLE days (
    date TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    location TEXT,
    work_start INTEGER,
    work_end INTEGER,
    note TEXT NOT NULL
) STRICT;

-- `project` may name a project that does not exist, as a day file may.
-- `end_minute` is past 24 * 60 for a block that ends on the next day.
CREATE TABLE blocks (
    date TEXT NOT NULL REFERENCES days (date) ON DELETE CASCADE,
    id TEXT NOT NULL,
    project TEXT NOT NULL,
    start_minute INTEGER NOT NULL,
    end_minute INTEGER NOT NULL,
    title TEXT NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (date, id)
) STRICT;

CREATE INDEX blocks_by_project ON blocks (project, date);

CREATE TABLE notes (
    project TEXT NOT NULL REFERENCES projects (slug) ON DELETE CASCADE,
    name TEXT NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (project, name)
) STRICT;

-- The global task list, `tasks.toml`.
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    created TEXT,
    due TEXT,
    done TEXT
) STRICT;
