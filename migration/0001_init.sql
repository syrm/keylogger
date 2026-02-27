CREATE TABLE keycount
(
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    duration_us INTEGER,                              -- Key press duration
    key_type    INTEGER CHECK (key_type IN (1, 2, 3)) -- 1: printable, 2: delete key, 3: other key
);