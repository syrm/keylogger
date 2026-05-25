use crate::client::deferred_drop::DeferredDrop;
use crate::shared::event::{KeyEvent, KeyType};
use evdevil::event::{EventType, Key};
use evdevil::{enumerate_hotplug, Evdev};
use nix::time::{clock_gettime, ClockId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::sync::mpsc::{Receiver, Sender};

const EV_KEY_RANGE: std::ops::RangeInclusive<u16> = 1..=248;
const EV_KEYUP: i32 = 0x00;
const EV_KEYDOWN: i32 = 0x01;

/// How often the REALTIME/BOOTTIME offset is recomputed. Since the device
/// clock is CLOCK_BOOTTIME (which keeps advancing during suspend, unlike
/// CLOCK_MONOTONIC), there's no abrupt jump to chase after a resume — only
/// slow, predictable NTP slew — so a fairly relaxed interval is enough.
const CLOCK_OFFSET_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Threshold used to detect events still timestamped with CLOCK_REALTIME
/// during the brief race window before `set_clockid(CLOCK_BOOTTIME)` takes
/// effect on a freshly opened device. A REALTIME timestamp (current epoch,
/// e.g. ~1.78e12 ms in 2026) is always orders of magnitude larger than a
/// realistic BOOTTIME value (time since boot, virtually never beyond a few
/// years ~ a few times 1e11 ms), so this scale gap is a reliable filter.
const REALTIME_LEAK_THRESHOLD_MS: u128 = 1_000_000_000_000; // ~ Sept. 2001

#[derive(Debug)]
pub(crate) struct KeyCounter {
    /// Shared, periodically refreshed offset (ms) between CLOCK_REALTIME
    /// and CLOCK_BOOTTIME, used to convert raw device timestamps (BOOTTIME)
    /// back into epoch ms for display, without ever touching the duration
    /// computation (which stays purely kernel-precision BOOTTIME deltas).
    clock_offset_ms: Arc<AtomicI64>,
}

impl KeyCounter {
    pub(crate) fn new() -> Self {
        Self {
            clock_offset_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    pub async fn monitor(&self, sender: Sender<KeyEvent>) -> anyhow::Result<()> {
        // Synchronous initial computation so we have a valid offset before
        // any device starts reporting events.
        let initial_offset = realtime_boottime_offset_ms()?;
        self.clock_offset_ms
            .store(initial_offset, Ordering::Relaxed);

        let (tx, rx) = tokio::sync::mpsc::channel::<Evdev>(100);

        let enumerate_devices = KeyCounter::enumerate_devices(tx);
        let dispatch_devices =
            KeyCounter::dispatch_devices(rx, sender, self.clock_offset_ms.clone());
        let refresh_offset = KeyCounter::refresh_clock_offset(self.clock_offset_ms.clone());

        tokio::try_join!(enumerate_devices, dispatch_devices, refresh_offset)?;

        Ok(())
    }

    /// Scans for keyboard devices, both already-plugged-in ones and future
    /// hotplugged ones, and forwards each one through `tx`.
    ///
    /// `enumerate_hotplug` is a blocking, synchronous iterator (it blocks the
    /// calling thread waiting for the next hotplug event), so this runs on a
    /// dedicated blocking thread via `spawn_blocking` rather than inside the
    /// async runtime.
    async fn enumerate_devices(tx: Sender<Evdev>) -> anyhow::Result<()> {
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            for res in enumerate_hotplug()? {
                let (path, evdev) = match res {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::error!(error = %e, "error during device enumeration");
                        continue;
                    }
                };

                if !KeyCounter::is_keyboard(&evdev) {
                    continue;
                }

                tracing::info!(path = ?path, "keyboard device added");
                if tx.blocking_send(evdev).is_err() {
                    // Receiver dropped, nothing more to do.
                    break;
                }
            }

            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Heuristic keyboard detection.
    ///
    /// Unlike the previous tokio_udev-based approach, evdevil's hotplug
    /// mechanism doesn't expose udev device properties (like
    /// `ID_INPUT_KEYBOARD`) to filter on, only a device path/handle. Checking
    /// for support of a representative letter key (`KEY_A`) alongside the
    /// `KEY` event type is a reasonable approximation of "this is a
    /// keyboard, not a mouse/power button/remote", but it's a cruder
    /// heuristic than udev's own classification.
    fn is_keyboard(evdev: &Evdev) -> bool {
        let Ok(events) = evdev.supported_events() else {
            return false;
        };
        if !events.contains(EventType::KEY) {
            return false;
        }

        let Ok(keys) = evdev.supported_keys() else {
            return false;
        };
        keys.contains(Key::KEY_A)
    }

    /// Spawns one independent task per keyboard device received from `rx`.
    ///
    /// There's no bookkeeping of running tasks by path here (unlike the
    /// previous implementation's `devices_task` map): evdevil doesn't send a
    /// "device removed" notification to react to, so each per-device task
    /// simply runs until its own event reads start failing (which happens
    /// once the device is actually gone) and then ends on its own.
    async fn dispatch_devices(
        mut rx: Receiver<Evdev>,
        sender: Sender<KeyEvent>,
        offset: Arc<AtomicI64>,
    ) -> anyhow::Result<()> {
        while let Some(evdev) = rx.recv().await {
            let sender = sender.clone();
            let offset = offset.clone();
            tokio::spawn(async move {
                KeyCounter::monitor_device(evdev, sender, offset).await;
            });
        }

        Ok(())
    }

    /// Periodically recomputes the CLOCK_REALTIME/CLOCK_BOOTTIME offset.
    ///
    /// Runs for the lifetime of the process. Because CLOCK_BOOTTIME already
    /// accounts for suspend time, the only thing this corrects for is slow
    /// NTP slew — staleness never grows beyond `CLOCK_OFFSET_REFRESH_INTERVAL`,
    /// regardless of how long the overall session runs.
    async fn refresh_clock_offset(offset: Arc<AtomicI64>) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(CLOCK_OFFSET_REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            match realtime_boottime_offset_ms() {
                Ok(o) => offset.store(o, Ordering::Relaxed),
                Err(e) => tracing::warn!(error = %e, "couldn't refresh clock offset"),
            }
        }
    }

    async fn monitor_device(evdev: Evdev, tx: Sender<KeyEvent>, offset: Arc<AtomicI64>) {
        // CLOCK_BOOTTIME: monotonic like CLOCK_MONOTONIC (so durations stay
        // immune to NTP adjustments and userspace scheduling jitter), but
        // unlike CLOCK_MONOTONIC it keeps advancing during suspend, so there
        // is no multi-hour jump to correct for after a resume.
        if let Err(e) = evdev.set_clockid(libc::CLOCK_BOOTTIME) {
            tracing::warn!(
                error = %e,
                "can't switch device clock to CLOCK_BOOTTIME, durations may be \
                 affected by wall-clock adjustments"
            );
        }

        let mut reader = match evdev.into_reader() {
            Ok(reader) => DeferredDrop::new(reader),
            Err(e) => {
                tracing::error!(error = %e, "can't create event reader");
                return;
            }
        };

        let mut events = match reader.async_events() {
            Ok(events) => events,
            Err(e) => {
                tracing::error!(error = %e, "can't read events asynchronously");
                return;
            }
        };

        let mut key_pressed: HashMap<u16, u128> = HashMap::new();

        loop {
            let event = match events.next_event().await {
                Ok(event) => event,
                Err(e) => {
                    tracing::info!(error = %e, "event stream ended, device likely disconnected");
                    break;
                }
            };

            if event.event_type() != EventType::KEY {
                continue;
            }

            if !EV_KEY_RANGE.contains(&event.raw_code()) {
                continue;
            }

            let Ok(ts) = event.time().duration_since(UNIX_EPOCH) else {
                continue;
            };
            let raw_boottime_ms = ts.as_millis();

            // Race window at device startup: a handful of events already
            // queued by the kernel before `set_clockid` took effect are
            // still timestamped with CLOCK_REALTIME. Discard them rather
            // than mixing clock domains within a keydown/keyup pair.
            if raw_boottime_ms >= REALTIME_LEAK_THRESHOLD_MS {
                tracing::debug!("discarding pre-switch (CLOCK_REALTIME) event");
                continue;
            }

            if event.raw_value() == EV_KEYDOWN {
                key_pressed.insert(event.raw_code(), raw_boottime_ms);
                continue;
            }

            if event.raw_value() == EV_KEYUP {
                let Some(raw_boottime_start) = key_pressed.remove(&event.raw_code()) else {
                    continue;
                };

                // Duration: raw kernel BOOTTIME delta — full precision,
                // unaffected by the offset or by NTP slew.
                let duration_ms = raw_boottime_ms.saturating_sub(raw_boottime_start);

                // Absolute timestamp: BOOTTIME reconstructed into epoch ms
                // via the periodically refreshed offset.
                let ts_ms_epoch = raw_boottime_start as i64 + offset.load(Ordering::Relaxed);

                let key_event = KeyEvent {
                    id: 0,
                    ts_ms: ts_ms_epoch,
                    duration_ms: duration_ms as i32,
                    key_type: match event.raw_code() {
                        2..=13 | 16..=27 | 30..=41 | 43..=53 | 57 | 71..=83 => KeyType::Typing,
                        14 => KeyType::Deletion,
                        111 => KeyType::Deletion,
                        _ => KeyType::Other,
                    },
                };

                if tx.send(key_event).await.is_err() {
                    // Receiver dropped, nothing more to forward — stop reading
                    // this device, mirroring the same shutdown behavior as
                    // `enumerate_devices`'s `tx.blocking_send(...).is_err()`.
                    tracing::info!("key event receiver dropped, stopping device monitor");
                    break;
                }
            }
        }
    }
}

/// Returns the offset (ms) such that `boottime_ms + offset == realtime_ms`,
/// i.e. how to convert a raw CLOCK_BOOTTIME timestamp into epoch ms.
fn realtime_boottime_offset_ms() -> anyhow::Result<i64> {
    let boottime_ms = read_clock_ms(ClockId::CLOCK_BOOTTIME)?;
    let realtime_ms = read_clock_ms(ClockId::CLOCK_REALTIME)?;
    Ok(realtime_ms - boottime_ms)
}

fn read_clock_ms(clock_id: ClockId) -> anyhow::Result<i64> {
    let ts = clock_gettime(clock_id)?;
    Ok(ts.tv_sec() * 1000 + ts.tv_nsec() / 1_000_000)
}
