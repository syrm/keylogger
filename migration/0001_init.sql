CREATE TABLE keycount
(
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    duration_us INTEGER -- Key press duration
);