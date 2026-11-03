use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ksni::{Tray, TrayMethods, menu::StandardItem};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    Show,
    Quit,
}

pub struct StudioTray {
    commands: UnboundedSender<TrayCommand>,
    available: Arc<AtomicBool>,
}

impl StudioTray {
    pub fn new(commands: UnboundedSender<TrayCommand>, available: Arc<AtomicBool>) -> Self {
        Self {
            commands,
            available,
        }
    }

    fn send(&self, command: TrayCommand) {
        let _ = self.commands.send(command);
    }
}

impl Tray for StudioTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").to_owned()
    }

    fn title(&self) -> String {
        "nowplaying".to_owned()
    }

    fn icon_name(&self) -> String {
        "audio-x-generic".to_owned()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let show = self.commands.clone();
        let quit = self.commands.clone();
        vec![
            StandardItem {
                label: "Open nowplaying".to_owned(),
                activate: Box::new(move |_| {
                    let _ = show.send(TrayCommand::Show);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".to_owned(),
                activate: Box::new(move |_| {
                    let _ = quit.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.available.store(true, Ordering::Relaxed);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.available.store(false, Ordering::Relaxed);
        true
    }
}

pub async fn start(
    commands: UnboundedSender<TrayCommand>,
    available: Arc<AtomicBool>,
) -> Result<ksni::Handle<StudioTray>, ksni::Error> {
    if std::env::var_os("FLATPAK_ID").is_some() {
        StudioTray::new(commands, available)
            .disable_dbus_name(true)
            .assume_sni_available(true)
            .spawn()
            .await
    } else {
        StudioTray::new(commands, available)
            .assume_sni_available(true)
            .spawn()
            .await
    }
}
