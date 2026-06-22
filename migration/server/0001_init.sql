create type keytype as enum ('TYPING', 'DELETION', 'OTHER');

comment on type keytype is 'TYPING = key producing a character (WPM calculation),
   DELETION = character removal (backspace/delete),
   OTHER = keys with no effect on text input (fn, print screen, scroll...)';

create table origin
(
    id                        integer generated always as identity
        constraint origin_pk
            primary key,
    name                      text                                   not null,
    public_key                text                                   not null,
    created_at                timestamp with time zone default now() not null,
    deleted_at                timestamp with time zone,
    last_sync_origin_event_id bigint,
    last_sync_date            timestamp with time zone
);

create table keyevent
(
    id              bigint generated always as identity
        constraint keyevent_pk
            primary key,
    pressed_at      timestamp with time zone not null,
    duration_ms     bigint                   not null,
    key_type        keytype,
    app_name        text                     not null,
    origin_id       integer                  not null
        constraint keyevent_origin_id_fk
            references origin
            on update cascade on delete cascade
            deferrable,
    origin_event_id bigint                   not null,
    constraint keyevent_origin_event_uq
        unique (origin_id, origin_event_id)
);

create table invitation
(
    code text not null
);

comment on column invitation.code is 'deleted when used';
