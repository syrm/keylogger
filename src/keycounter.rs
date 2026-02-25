use evdev::{Device, EventStream};
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio_udev::{AsyncMonitorSocket, EventType, MonitorBuilder};
use tracing::field;

const EV_KEY_RANGE: std::ops::RangeInclusive<u16> = 1..=248;
const EV_KEYUP: i32 = 0x00;
const EV_KEYDOWN: i32 = 0x01;

#[derive(Debug)]
pub(crate) struct KeyCounter {}

#[derive(Debug)]
pub(crate) struct KeyEvent {
    pub ts_ms: u64,
    pub duration_us: u128,
}

#[derive(Debug, PartialEq)]
enum DeviceEventAction {
    Add,
    Remove,
}

#[derive(Debug)]
struct DeviceEvent {
    path: String,
    action: DeviceEventAction,
}

impl KeyCounter {
    pub(crate) fn new() -> Self {
        Self {}
    }

    pub async fn monitor(&self, sender: Sender<KeyEvent>) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::mpsc::channel::<DeviceEvent>(100);

        let monitor_keyboards = KeyCounter::monitor_keyboards(tx);
        let monitor_keyboard = KeyCounter::monitor_keyboard(rx, sender);
        tokio::try_join!(monitor_keyboards, monitor_keyboard)?;

        Ok(())
    }

    async fn monitor_keyboards(tx: Sender<DeviceEvent>) -> anyhow::Result<()> {
        loop {
            let devices = evdev::enumerate().collect::<Vec<_>>();
            KeyCounter::process_devices(tx.clone(), &devices).await?;

            // process devices

            let monitor = MonitorBuilder::new()?.match_subsystem("input")?.listen()?;

            let mut socket = AsyncMonitorSocket::new(monitor)?;

            while let Some(event) = socket.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        tracing::error!(error = %error, "udev error");
                        continue;
                    }
                };

                let Some(devnode) = event.devnode() else {
                    continue;
                };

                if event.property_value("ID_INPUT_KEYBOARD") == None {
                    continue;
                }

                let event_type = event.event_type();
                if !matches!(event_type, EventType::Add | EventType::Remove) {
                    continue;
                }

                match event_type {
                    EventType::Add => {
                        tracing::info!(path = ?devnode.to_string_lossy(), "device added");
                        tx.send(DeviceEvent {
                            action: DeviceEventAction::Add,
                            path: devnode.to_string_lossy().to_string(),
                        })
                        .await?;
                    }
                    EventType::Remove => {
                        tracing::info!(path = ?devnode.to_string_lossy(), "device removed");
                        tx.send(DeviceEvent {
                            action: DeviceEventAction::Remove,
                            path: devnode.to_string_lossy().to_string(),
                        })
                        .await?;
                    }
                    _ => {}
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    async fn task_monitor_keyboard(path: String, tx: Sender<KeyEvent>) {
        'reconnect: loop {
            let device = match Device::open(&path) {
                Err(e) => {
                    tracing::error!(error = %e, "can't open device");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    break;
                }
                Ok(dev) => dev,
            };

            let mut stream = match device.into_event_stream() {
                Err(e) => {
                    tracing::error!(error = %e, "can't convert device to event stream");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue 'reconnect;
                }
                Ok(stream) => stream,
            };

            KeyCounter::monitor_keyboard_events(stream, tx.clone()).await;
        }
    }

    async fn monitor_keyboard(
        mut rx: Receiver<DeviceEvent>,
        sender: Sender<KeyEvent>,
    ) -> anyhow::Result<()> {
        let mut devices_task: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

        while let Some(device_event) = rx.recv().await {
            let path: String = device_event.path;

            if device_event.action == DeviceEventAction::Remove {
                if let Some(task) = devices_task.get(&path) {
                    task.abort();
                }
                devices_task.remove(&path).unwrap().abort();
                continue;
            }

            if devices_task.contains_key(&path) {
                continue;
            }

            let sender = sender.clone();

            devices_task.insert(
                path.clone(),
                tokio::spawn(async move {
                    KeyCounter::task_monitor_keyboard(path, sender).await;
                }),
            );
        }

        Ok(())
    }

    async fn monitor_keyboard_events(mut stream: EventStream, tx: Sender<KeyEvent>) {
        let mut key_pressed: HashMap<u16, u128> = HashMap::new();

        loop {
            match stream.next_event().await {
                Ok(event) => {
                    if event.event_type() != evdev::EventType::KEY {
                        continue;
                    }

                    if !EV_KEY_RANGE.contains(&event.code()) {
                        continue;
                    };

                    let Ok(ts) = event.timestamp().duration_since(UNIX_EPOCH) else {
                        continue;
                    };

                    if event.value() == EV_KEYDOWN {
                        key_pressed.insert(event.code(), ts.as_micros());
                        continue;
                    }

                    if event.value() == EV_KEYUP {
                        let Some(ts_us_start) = key_pressed.remove(&event.code()) else {
                            continue;
                        };

                        let duration_us = ts.as_micros() - ts_us_start;
                        tracing::info!(ts_us_start = field::Empty, "key pressed");
                        tx.send(KeyEvent {
                            ts_ms: (ts_us_start / 1000) as u64,
                            duration_us: duration_us,
                        })
                        .await
                        .ok();
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "event stream error, reconnecting...");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    break;
                }
            }
        }
    }

    async fn process_devices(
        tx: Sender<DeviceEvent>,
        devices: &[(PathBuf, Device)],
    ) -> anyhow::Result<()> {
        for (path, device) in devices {
            tracing::info!(path = ?path, "device added");
            tx.send(DeviceEvent {
                action: DeviceEventAction::Add,
                path: path.to_string_lossy().to_string(),
            })
            .await?;
        }

        Ok(())
    }
}
