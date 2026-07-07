use crate::shared::event::{KeyEvent, KeyEventsPayload, KeyEventsPayloadWire};

pub(crate) fn encode(payload: &KeyEventsPayload) -> anyhow::Result<Vec<u8>> {
    let wire = to_wire(payload);
    let bytes = postcard::to_allocvec(&wire)?;
    let compressed = zstd::stream::encode_all(&bytes[..], 22)?;
    Ok(compressed)
}

pub(crate) fn decode(compressed: &[u8]) -> anyhow::Result<KeyEventsPayload> {
    let bytes = zstd::stream::decode_all(compressed)?;
    let wire: KeyEventsPayloadWire = postcard::from_bytes(&bytes)?;
    from_wire(wire)
}

fn to_wire(payload: &KeyEventsPayload) -> KeyEventsPayloadWire {
    let n = payload.events.len();
    let mut id_deltas = Vec::with_capacity(n);
    let mut ts_deltas = Vec::with_capacity(n);
    let mut durations_ms = Vec::with_capacity(n);
    let mut key_types = Vec::with_capacity(n);
    let mut app_names = Vec::with_capacity(n);

    let (mut prev_id, mut prev_ts) = (0i64, 0i64);

    for e in &payload.events {
        id_deltas.push(e.id - prev_id);
        ts_deltas.push(e.ts_ms - prev_ts);
        durations_ms.push(e.duration_ms);
        key_types.push(e.key_type);
        app_names.push(e.app_name.clone());

        prev_id = e.id;
        prev_ts = e.ts_ms;
    }

    KeyEventsPayloadWire {
        origin_id: payload.origin_id,
        issued_at: payload.issued_at,
        signature: payload.signature.clone(),
        id_deltas,
        ts_deltas,
        durations_ms,
        key_types,
        app_names,
    }
}

fn from_wire(wire: KeyEventsPayloadWire) -> anyhow::Result<KeyEventsPayload> {
    let n = wire.id_deltas.len();
    if wire.ts_deltas.len() != n
        || wire.durations_ms.len() != n
        || wire.key_types.len() != n
        || wire.app_names.len() != n
    {
        anyhow::bail!("malformed payload: vector length mismatch");
    }

    let mut events = Vec::with_capacity(n);
    let (mut id, mut ts_ms) = (0i64, 0i64);

    for i in 0..n {
        id += wire.id_deltas[i];
        ts_ms += wire.ts_deltas[i];
        events.push(KeyEvent {
            id,
            ts_ms,
            duration_ms: wire.durations_ms[i],
            key_type: wire.key_types[i],
            app_name: wire.app_names[i].clone(),
        });
    }

    Ok(KeyEventsPayload {
        origin_id: wire.origin_id,
        issued_at: wire.issued_at,
        signature: wire.signature,
        events,
    })
}
