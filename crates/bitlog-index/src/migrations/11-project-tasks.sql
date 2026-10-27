-- Projects have task lists of their own, `projects/<slug>/tasks.toml`, next
-- to the global one, whose tasks have no `project`. Task ids are unique only
-- within their list.

DROP TABLE tasks_search;
DROP TABLE tasks;

CREATE TABLE tasks (
    project TEXT REFERENCES projects (slug) ON DELETE CASCADE,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    created TEXT,
    due TEXT,
    done TEXT,
    UNIQUE (project, id)
) STRICT;

CREATE VIRTUAL TABLE tasks_search USING fts5 (
    title, content = 'tasks', tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER tasks_search_insert AFTER INSERT ON tasks BEGIN
    INSERT INTO tasks_search (rowid, title) VALUES (new.rowid, new.title);
END;
CREATE TRIGGER tasks_search_delete AFTER DELETE ON tasks BEGIN
    INSERT INTO tasks_search (tasks_search, rowid, title) VALUES ('delete', old.rowid, old.title);
END;
CREATE TRIGGER tasks_search_update AFTER UPDATE ON tasks BEGIN
    INSERT INTO tasks_search (tasks_search, rowid, title) VALUES ('delete', old.rowid, old.title);
    INSERT INTO tasks_search (rowid, title) VALUES (new.rowid, new.title);
END;

-- Read the global task list again.
DELETE FROM files WHERE path = 'tasks.toml';
