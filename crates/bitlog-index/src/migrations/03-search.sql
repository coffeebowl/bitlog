-- Full-text search over block titles and texts, day notes, notes and tasks.
--
-- One FTS5 table per content table, reading its text from there
-- (`content=`) and linked to it by rowid. Triggers keep them in step. The
-- content tables have no INTEGER PRIMARY KEY, so their rowids must stay as
-- they are: never VACUUM the index, rebuild it instead.
--
-- The trigram tokenizer finds any part of a word of three characters or
-- more, ignoring case and accents.

CREATE VIRTUAL TABLE blocks_search USING fts5 (
    title, text, content = 'blocks', tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER blocks_search_insert AFTER INSERT ON blocks BEGIN
    INSERT INTO blocks_search (rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;
CREATE TRIGGER blocks_search_delete AFTER DELETE ON blocks BEGIN
    INSERT INTO blocks_search (blocks_search, rowid, title, text)
    VALUES ('delete', old.rowid, old.title, old.text);
END;
CREATE TRIGGER blocks_search_update AFTER UPDATE ON blocks BEGIN
    INSERT INTO blocks_search (blocks_search, rowid, title, text)
    VALUES ('delete', old.rowid, old.title, old.text);
    INSERT INTO blocks_search (rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;

CREATE VIRTUAL TABLE days_search USING fts5 (
    note, content = 'days', tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER days_search_insert AFTER INSERT ON days BEGIN
    INSERT INTO days_search (rowid, note) VALUES (new.rowid, new.note);
END;
CREATE TRIGGER days_search_delete AFTER DELETE ON days BEGIN
    INSERT INTO days_search (days_search, rowid, note) VALUES ('delete', old.rowid, old.note);
END;
CREATE TRIGGER days_search_update AFTER UPDATE ON days BEGIN
    INSERT INTO days_search (days_search, rowid, note) VALUES ('delete', old.rowid, old.note);
    INSERT INTO days_search (rowid, note) VALUES (new.rowid, new.note);
END;

CREATE VIRTUAL TABLE notes_search USING fts5 (
    name, text, content = 'notes', tokenize = 'trigram remove_diacritics 1'
);
CREATE TRIGGER notes_search_insert AFTER INSERT ON notes BEGIN
    INSERT INTO notes_search (rowid, name, text) VALUES (new.rowid, new.name, new.text);
END;
CREATE TRIGGER notes_search_delete AFTER DELETE ON notes BEGIN
    INSERT INTO notes_search (notes_search, rowid, name, text)
    VALUES ('delete', old.rowid, old.name, old.text);
END;
CREATE TRIGGER notes_search_update AFTER UPDATE ON notes BEGIN
    INSERT INTO notes_search (notes_search, rowid, name, text)
    VALUES ('delete', old.rowid, old.name, old.text);
    INSERT INTO notes_search (rowid, name, text) VALUES (new.rowid, new.name, new.text);
END;

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

-- Content indexed before this migration.
INSERT INTO blocks_search (blocks_search) VALUES ('rebuild');
INSERT INTO days_search (days_search) VALUES ('rebuild');
INSERT INTO notes_search (notes_search) VALUES ('rebuild');
INSERT INTO tasks_search (tasks_search) VALUES ('rebuild');
