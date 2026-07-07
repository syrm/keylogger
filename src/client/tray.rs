use ksni::Tray;
use tokio::sync::mpsc::Sender;

#[derive(Debug)]
pub(crate) struct MyTray {
    tx: Sender<bool>,
    show: bool,
}

impl MyTray {
    pub(crate) fn new(tx: Sender<bool>) -> Self {
        Self { tx, show: false }
    }
}

impl Tray for MyTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "Mon App".to_string()
    }

    fn icon_name(&self) -> String {
        "application-x-executable".to_string()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![ksni::MenuItem::Standard(ksni::menu::StandardItem {
            label: "Show/Hide".to_string(),
            activate: Box::new(|tray: &mut MyTray| {
                let tx = tray.tx.clone();
                tray.show = !tray.show;
                let show = tray.show;
                println!("show: {}", show);
                tokio::spawn(async move {
                    tx.send(show).await.expect("TODO: ui menu");
                });
            }),
            ..Default::default()
        })]
    }
}
