CREATE TABLE keyevent
(
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,                      -- Key press duration
    key_type    INTEGER CHECK (key_type IN (1, 2, 3)), -- 1: typing key, 2: deletion key, 3: other key
    app_name    TEXT    NOT NULL
);

CREATE TABLE metadata
(
    key   TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);

INSERT INTO metadata (key, value)
VALUES ('last_event_id_synced', 0);