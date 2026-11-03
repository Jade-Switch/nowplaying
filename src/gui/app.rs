use std::{
    fmt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    artwork::ArtworkCache,
    mpris::{PlayerSelection, PlayerSelector},
    playback::{PlaybackSnapshot, PlaybackStatus, PlayerInfo},
    theme::{LoadedTheme, ThemeDiscovery, ThemeManager},
};
use iced::{
    Alignment, Element, Length, Subscription, Task, Theme, time,
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input,
    },
    window,
};
use tokio::io::AsyncWriteExt;

use super::design::{self, ConnectionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Themes,
    Playback,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlayerChoice {
    Automatic,
    Manual { bus_name: String, identity: String },
}

impl PlayerChoice {
    fn from_player(player: &PlayerInfo) -> Self {
        Self::Manual {
            bus_name: player.bus_name.clone(),
            identity: player.identity.clone(),
        }
    }

    fn selection(&self) -> PlayerSelection {
        match self {
            Self::Automatic => PlayerSelection::Automatic,
            Self::Manual { bus_name, .. } => PlayerSelection::Manual(bus_name.clone()),
        }
    }
}

impl fmt::Display for PlayerChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Automatic => f.write_str("Automatic selection"),
            Self::Manual { identity, .. } => f.write_str(identity),
        }
    }
}

impl Section {
    const fn label(self) -> &'static str {
        match self {
            Self::Themes => "Themes",
            Self::Playback => "Playback",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Clone)]
pub struct GuiContext {
    pub themes: ThemeManager,
    pub catalog: ThemeDiscovery,
    pub active_theme: Option<String>,
    pub playback_state: Option<tokio::sync::watch::Receiver<PlaybackSnapshot>>,
    pub players_state: Option<tokio::sync::watch::Receiver<Vec<PlayerInfo>>>,
    pub player_selector: Option<PlayerSelector>,
    pub artwork_cache: Arc<ArtworkCache>,
    pub service_url: String,
    pub settings_path: PathBuf,
    pub themes_dir: PathBuf,
    pub restart_requested: Arc<AtomicBool>,
    pub tray_available: Arc<AtomicBool>,
    pub show_requested: Arc<AtomicBool>,
    pub shutdown_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Section),
    SelectTheme(String),
    ThemeSelected(Result<String, String>),
    RefreshThemes,
    ThemesRefreshed(Result<ThemeDiscovery, String>),
    ImportTheme,
    ImportFileChosen(Option<PathBuf>),
    ThemeImported(Result<ImportedTheme, String>),
    OpenTheme(String),
    PreviewTheme(String),
    ThemePreviewed(Result<String, String>),
    RequestDeleteTheme(String),
    CancelDeleteTheme,
    DeleteTheme(String),
    ThemeDeleted(Result<(String, ThemeDiscovery), String>),
    CopyServiceUrl,
    OpenThemesFolder,
    SystemPathOpened(Result<PathBuf, String>),
    ServerPortChanged(String),
    RestartWithPort,
    ServerPortSaved(Result<u16, String>),
    StartupToggled(bool),
    StartupSaved(Result<bool, String>),
    CloseRequested(window::Id),
    WindowOpened(window::Id),
    SelectPlayer(PlayerChoice),
    ArtworkLoaded(ArtworkKey, Option<Vec<u8>>),
    Tick,
}

#[derive(Debug, Clone)]
struct ImportedTheme {
    id: String,
    catalog: ThemeDiscovery,
    source: PathBuf,
    destination: PathBuf,
    source_removal_error: Option<String>,
}

type ArtworkKey = (Option<String>, Option<String>);

static SETTINGS_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct App {
    section: Section,
    themes: ThemeManager,
    catalog: ThemeDiscovery,
    active_theme: Option<String>,
    playback_state: Option<tokio::sync::watch::Receiver<PlaybackSnapshot>>,
    players_state: Option<tokio::sync::watch::Receiver<Vec<PlayerInfo>>>,
    players: Vec<PlayerInfo>,
    player_selector: Option<PlayerSelector>,
    selected_player: PlayerChoice,
    artwork_cache: Arc<ArtworkCache>,
    artwork_key: Option<ArtworkKey>,
    artwork: Option<iced::widget::image::Handle>,
    playback: PlaybackSnapshot,
    service_url: String,
    service_url_copied: bool,
    settings_path: PathBuf,
    themes_dir: PathBuf,
    server_port_input: String,
    start_on_startup: bool,
    restart_requested: Arc<AtomicBool>,
    tray_available: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    status_message: Option<String>,
    confirming_delete: Option<String>,
    shutdown_requested: Arc<AtomicBool>,
    window_id: Option<window::Id>,
}

impl App {
    fn new(context: GuiContext) -> Self {
        let GuiContext {
            themes,
            catalog,
            active_theme,
            playback_state,
            players_state,
            player_selector,
            artwork_cache,
            service_url,
            settings_path,
            themes_dir,
            restart_requested,
            tray_available,
            show_requested,
            shutdown_requested,
        } = context;
        let playback = playback_state
            .as_ref()
            .map(|state| state.borrow().clone())
            .unwrap_or_default();
        let players = players_state
            .as_ref()
            .map(|players| players.borrow().clone())
            .unwrap_or_default();
        let server_port_input = url::Url::parse(&service_url)
            .ok()
            .and_then(|url| url.port())
            .unwrap_or(9005)
            .to_string();
        let start_on_startup =
            autostart_file_path(&settings_path).is_some_and(|path| path.exists());
        Self {
            section: Section::Themes,
            themes,
            catalog,
            active_theme,
            playback_state,
            players_state,
            players,
            player_selector,
            selected_player: PlayerChoice::Automatic,
            artwork_cache,
            artwork_key: None,
            artwork: None,
            playback,
            service_url,
            service_url_copied: false,
            settings_path,
            themes_dir,
            server_port_input,
            start_on_startup,
            restart_requested,
            tray_available,
            show_requested,
            status_message: None,
            confirming_delete: None,
            shutdown_requested,
            window_id: None,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(section) => {
                self.section = section;
                Task::none()
            }
            Message::SelectTheme(id) => {
                let themes = self.themes.clone();
                Task::perform(
                    async move {
                        themes
                            .select(&id)
                            .await
                            .map(|theme| theme.id().to_owned())
                            .map_err(|error| error.to_string())
                    },
                    Message::ThemeSelected,
                )
            }
            Message::ThemeSelected(result) => {
                match result {
                    Ok(id) => {
                        self.active_theme = Some(id);
                        self.status_message = None;
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::RefreshThemes => {
                let themes = self.themes.clone();
                Task::perform(
                    async move { themes.refresh().await.map_err(|error| error.to_string()) },
                    Message::ThemesRefreshed,
                )
            }
            Message::ThemesRefreshed(result) => {
                match result {
                    Ok(catalog) => {
                        self.catalog = catalog;
                        self.status_message = None;
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::ImportTheme => Task::perform(pick_theme_file(), Message::ImportFileChosen),
            Message::ImportFileChosen(Some(path)) => {
                let themes = self.themes.clone();
                Task::perform(
                    async move {
                        let theme = themes
                            .import_file(&path)
                            .await
                            .map_err(|error| error.to_string())?;
                        let id = theme.id().to_owned();
                        let destination = theme
                            .source_path
                            .clone()
                            .ok_or_else(|| "Imported theme has no storage path".to_owned())?;
                        let source_removal_error = tokio::fs::remove_file(&path)
                            .await
                            .err()
                            .map(|error| error.to_string());
                        let catalog = themes.catalog().await;
                        Ok(ImportedTheme {
                            id,
                            catalog,
                            source: path,
                            destination,
                            source_removal_error,
                        })
                    },
                    Message::ThemeImported,
                )
            }
            Message::ImportFileChosen(None) => {
                self.status_message = Some(
                    "No theme was selected. If the picker did not open, check kdialog or the desktop file portal."
                        .to_owned(),
                );
                Task::none()
            }
            Message::ThemeImported(result) => {
                match result {
                    Ok(imported) => {
                        let name = imported
                            .catalog
                            .themes
                            .get(&imported.id)
                            .map(|theme| theme.document.theme.name.clone())
                            .unwrap_or_else(|| imported.id.clone());
                        self.catalog = imported.catalog;
                        self.status_message = Some(match imported.source_removal_error {
                            Some(error) => format!(
                                "Imported {name} to {} but couldn't remove the original at {}: {error}",
                                imported.destination.display(),
                                imported.source.display()
                            ),
                            None => format!("Moved {name} to {}.", imported.destination.display()),
                        });
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::OpenTheme(id) => {
                let Some(path) = self
                    .catalog
                    .themes
                    .get(&id)
                    .and_then(|theme| theme.source_path.clone())
                else {
                    self.status_message = Some("This theme has no editable file.".to_owned());
                    return Task::none();
                };
                open_system_path(path)
            }
            Message::PreviewTheme(id) => {
                let mut url = match url::Url::parse(&self.service_url) {
                    Ok(url) => url,
                    Err(error) => {
                        self.status_message = Some(format!("Invalid local server URL: {error}"));
                        return Task::none();
                    }
                };
                let Ok(mut segments) = url.path_segments_mut() else {
                    self.status_message = Some("Could not build the theme preview URL.".to_owned());
                    return Task::none();
                };
                segments.clear().push("preview").push(&id);
                drop(segments);
                let url = url.to_string();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            Command::new("xdg-open")
                                .arg(&url)
                                .spawn()
                                .map(|_| url)
                                .map_err(|error| error.to_string())
                        })
                        .await
                        .map_err(|error| error.to_string())?
                    },
                    Message::ThemePreviewed,
                )
            }
            Message::ThemePreviewed(result) => {
                self.status_message = Some(match result {
                    Ok(url) => format!("Opened theme preview at {url}."),
                    Err(error) => format!("Could not open theme preview: {error}"),
                });
                Task::none()
            }
            Message::RequestDeleteTheme(id) => {
                self.confirming_delete = Some(id);
                Task::none()
            }
            Message::CancelDeleteTheme => {
                self.confirming_delete = None;
                Task::none()
            }
            Message::DeleteTheme(id) => {
                let themes = self.themes.clone();
                Task::perform(
                    async move {
                        themes
                            .delete(&id)
                            .await
                            .map_err(|error| error.to_string())?;
                        Ok((id, themes.catalog().await))
                    },
                    Message::ThemeDeleted,
                )
            }
            Message::ThemeDeleted(result) => {
                self.confirming_delete = None;
                match result {
                    Ok((id, catalog)) => {
                        self.catalog = catalog;
                        self.status_message = Some(format!("Deleted theme {id}."));
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::CopyServiceUrl => {
                self.service_url_copied = true;
                iced::clipboard::write(self.service_url.clone())
            }
            Message::OpenThemesFolder => open_system_path(self.themes_dir.clone()),
            Message::SystemPathOpened(result) => {
                self.status_message = Some(match result {
                    Ok(path) => format!("Opened {}.", path.display()),
                    Err(error) => format!("Could not open that location: {error}"),
                });
                Task::none()
            }
            Message::ServerPortChanged(value) => {
                self.server_port_input = value;
                Task::none()
            }
            Message::RestartWithPort => {
                let port = match self.server_port_input.parse::<u16>() {
                    Ok(port) if port != 0 => port,
                    _ => {
                        self.status_message =
                            Some("Enter a valid port number between 1 and 65535.".to_owned());
                        return Task::none();
                    }
                };
                let settings_path = self.settings_path.clone();
                Task::perform(
                    async move {
                        save_server_port(settings_path, port).await?;
                        Ok(port)
                    },
                    Message::ServerPortSaved,
                )
            }
            Message::ServerPortSaved(result) => {
                match result {
                    Ok(port) => {
                        self.status_message =
                            Some(format!("Restarting Now Playing Studio on port {port}."));
                        self.restart_requested.store(true, Ordering::Relaxed);
                        self.shutdown_requested.store(true, Ordering::Relaxed);
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::StartupToggled(enabled) => {
                let settings_path = self.settings_path.clone();
                Task::perform(
                    async move { set_autostart(&settings_path, enabled).await },
                    Message::StartupSaved,
                )
            }
            Message::StartupSaved(result) => {
                match result {
                    Ok(enabled) => {
                        self.start_on_startup = enabled;
                        self.status_message = Some(if enabled {
                            "Now Playing Studio will start when you sign in.".to_owned()
                        } else {
                            "Now Playing Studio will no longer start when you sign in.".to_owned()
                        });
                    }
                    Err(error) => self.status_message = Some(error),
                }
                Task::none()
            }
            Message::CloseRequested(id) => {
                if self.tray_available.load(Ordering::Relaxed) {
                    self.window_id = None;
                    window::close(id)
                } else {
                    iced::exit()
                }
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                Task::none()
            }
            Message::SelectPlayer(choice) => {
                if let Some(selector) = &self.player_selector {
                    selector.set_selection(choice.selection());
                    self.selected_player = choice;
                }
                Task::none()
            }
            Message::ArtworkLoaded(key, bytes) => {
                if self.artwork_key.as_ref() == Some(&key) {
                    self.artwork = bytes.map(iced::widget::image::Handle::from_bytes);
                }
                Task::none()
            }
            Message::Tick => {
                if self.shutdown_requested.load(Ordering::Relaxed) {
                    return iced::exit();
                }
                if let Some(state) = &self.playback_state {
                    self.playback = state.borrow().clone();
                }
                if let Some(players) = &self.players_state {
                    self.players = players.borrow().clone();
                }
                let mut tasks = Vec::new();
                let artwork_key = (
                    self.playback.metadata.track_id.clone(),
                    self.playback.metadata.artwork_uri.clone(),
                );
                if self.artwork_key.as_ref() != Some(&artwork_key) {
                    self.artwork_key = Some(artwork_key.clone());
                    self.artwork = None;
                    if artwork_key.1.is_some() {
                        let cache = Arc::clone(&self.artwork_cache);
                        let key = artwork_key;
                        let uri = key.1.clone();
                        tasks.push(Task::perform(
                            async move { load_artwork(cache, uri).await },
                            move |bytes| Message::ArtworkLoaded(key, bytes),
                        ));
                    }
                }
                if self.show_requested.swap(false, Ordering::Relaxed) {
                    if let Some(id) = self.window_id {
                        tasks.push(window::gain_focus(id));
                    } else {
                        let (id, open) = window::open(window_settings());
                        self.window_id = Some(id);
                        tasks.push(open.map(Message::WindowOpened));
                    }
                }
                Task::batch(tasks)
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            time::every(Duration::from_millis(500)).map(|_| Message::Tick),
            window::close_requests().map(Message::CloseRequested),
        ])
    }

    fn player_choices(&self) -> Vec<PlayerChoice> {
        let mut choices = Vec::with_capacity(self.players.len() + 2);
        choices.push(PlayerChoice::Automatic);
        choices.extend(self.players.iter().map(PlayerChoice::from_player));
        if let PlayerChoice::Manual { bus_name, .. } = &self.selected_player
            && !self
                .players
                .iter()
                .any(|player| player.bus_name == *bus_name)
        {
            choices.push(self.selected_player.clone());
        }
        choices
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar = self.sidebar();
        let page = match self.section {
            Section::Themes => self.themes_page(),
            Section::Playback => self.playback_page(),
            Section::Settings => self.settings_page(),
        };
        container(
            row![sidebar, page]
                .spacing(design::spacing::LG)
                .height(Length::Fill),
        )
        .padding(design::spacing::LG)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| design::window_surface())
        .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let nav = [Section::Themes, Section::Playback, Section::Settings]
            .into_iter()
            .fold(column!().spacing(design::spacing::SM), |nav, section| {
                let selected = self.section == section;
                nav.push(
                    button(text(section.label()).size(design::typography::BODY_LARGE))
                        .width(Length::Fill)
                        .padding([design::spacing::MD, design::spacing::LG])
                        .style(move |theme, status| {
                            design::navigation_button(theme, status, selected)
                        })
                        .on_press(Message::Navigate(section)),
                )
            });

        container(
            column![
                text("NOW PLAYING")
                    .size(design::typography::CAPTION)
                    .color(design::LAVENDER),
                text("Studio")
                    .size(design::typography::TITLE)
                    .color(design::TEXT_PRIMARY),
                space::vertical().height(design::spacing::XL),
                nav,
                space::vertical().height(Length::Fill),
                row![
                    container(text(" "))
                        .width(8)
                        .height(8)
                        .style(|_| design::status_indicator(ConnectionState::Connected)),
                    text("LOCAL SERVER RUNNING")
                        .size(design::typography::CAPTION)
                        .color(design::STATUS_CONNECTED),
                ]
                .spacing(design::spacing::SM)
                .align_y(Alignment::Center),
                text("OBS BROWSER SOURCE")
                    .size(10)
                    .color(design::TEXT_SECONDARY),
                row![
                    text(&self.service_url)
                        .size(design::typography::CAPTION)
                        .color(design::LAVENDER_SOFT)
                        .width(Length::Fill),
                    button(text(if self.service_url_copied {
                        "Copied"
                    } else {
                        "Copy"
                    }))
                    .padding([design::spacing::XS, design::spacing::SM])
                    .style(design::secondary_button)
                    .on_press(Message::CopyServiceUrl),
                ]
                .spacing(design::spacing::XS)
                .align_y(Alignment::Center),
            ]
            .spacing(design::spacing::XS)
            .height(Length::Fill),
        )
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .padding(design::spacing::LG)
        .style(|_| design::surface())
        .into()
    }

    fn themes_page(&self) -> Element<'_, Message> {
        let header = row![
            column![
                text("Themes")
                    .size(design::typography::TITLE)
                    .color(design::TEXT_PRIMARY),
                text("Choose the look for your OBS Browser Source.")
                    .size(design::typography::BODY)
                    .color(design::TEXT_SECONDARY),
            ]
            .spacing(design::spacing::XS),
            space::horizontal().width(Length::Fill),
            button(text("Import theme").size(design::typography::BODY))
                .padding([design::spacing::SM, design::spacing::LG])
                .style(design::secondary_button)
                .on_press(Message::ImportTheme),
            button(text("Refresh").size(design::typography::BODY))
                .padding([design::spacing::SM, design::spacing::LG])
                .style(design::secondary_button)
                .on_press(Message::RefreshThemes),
        ]
        .align_y(Alignment::Center);

        let mut cards = column!().spacing(design::spacing::MD);
        for (id, theme) in &self.catalog.themes {
            cards = cards.push(self.theme_card(id, theme));
        }
        for issue in &self.catalog.issues {
            cards = cards.push(self.invalid_theme_card(issue));
        }
        if self.catalog.themes.is_empty() && self.catalog.issues.is_empty() {
            cards = cards.push(
                container(text("No valid themes found.").color(design::TEXT_SECONDARY))
                    .padding(design::spacing::XL)
                    .width(Length::Fill)
                    .style(|_| design::surface()),
            );
        }

        let footer = self.status_message.as_ref().map(|message| {
            text(message.as_str())
                .size(design::typography::BODY)
                .color(design::STATUS_ERROR)
        });
        let content = column![header, scrollable(cards).height(Length::Fill)]
            .spacing(design::spacing::XL)
            .height(Length::Fill);
        container(if let Some(message) = footer {
            column![content, message].spacing(design::spacing::SM)
        } else {
            content
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn settings_page(&self) -> Element<'_, Message> {
        let content = column![
            column![
                text("Settings")
                    .size(design::typography::TITLE)
                    .color(design::TEXT_PRIMARY),
                text("Change the local server port, startup behavior, and installed themes.")
                    .size(design::typography::BODY)
                    .color(design::TEXT_SECONDARY),
            ]
            .spacing(design::spacing::XS),
            container(
                column![
                    text("SERVER PORT")
                        .size(design::typography::CAPTION)
                        .color(design::TEXT_SECONDARY),
                    row![
                        text_input("9005", &self.server_port_input)
                            .on_input(Message::ServerPortChanged)
                            .width(Length::Fixed(140.0)),
                        button(text("Save and restart"))
                            .padding([design::spacing::SM, design::spacing::LG])
                            .style(design::primary_button)
                            .on_press(Message::RestartWithPort),
                    ]
                    .spacing(design::spacing::SM)
                    .align_y(Alignment::Center),
                    text("Changing the port saves settings.toml and restarts the app.")
                        .size(design::typography::CAPTION)
                        .color(design::TEXT_SECONDARY),
                ]
                .spacing(design::spacing::SM),
            )
            .padding(design::spacing::LG)
            .width(Length::Fill)
            .style(|_| design::surface()),
            container(
                column![
                    text("STARTUP")
                        .size(design::typography::CAPTION)
                        .color(design::TEXT_SECONDARY),
                    checkbox(self.start_on_startup)
                        .label("Start Now Playing Studio when I sign in")
                        .on_toggle(Message::StartupToggled),
                    text("The app will open in the background with its tray icon.")
                        .size(design::typography::CAPTION)
                        .color(design::TEXT_SECONDARY),
                ]
                .spacing(design::spacing::SM),
            )
            .padding(design::spacing::LG)
            .width(Length::Fill)
            .style(|_| design::surface()),
            container(
                column![
                    text("THEME DIRECTORY")
                        .size(design::typography::CAPTION)
                        .color(design::TEXT_SECONDARY),
                    text(self.themes_dir.display().to_string())
                        .size(design::typography::BODY)
                        .color(design::TEXT_PRIMARY),
                    button(text("Open themes folder"))
                        .padding([design::spacing::SM, design::spacing::LG])
                        .style(design::secondary_button)
                        .on_press(Message::OpenThemesFolder),
                ]
                .spacing(design::spacing::SM),
            )
            .padding(design::spacing::LG)
            .width(Length::Fill)
            .style(|_| design::surface()),
        ]
        .spacing(design::spacing::XL)
        .width(Length::Fill);

        container(if let Some(message) = &self.status_message {
            column![
                content,
                text(message.as_str()).color(design::TEXT_SECONDARY)
            ]
            .spacing(design::spacing::SM)
        } else {
            content
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn theme_card<'a>(&self, id: &'a str, theme: &'a LoadedTheme) -> Element<'a, Message> {
        let is_active = self.active_theme.as_deref() == Some(id);
        let select = button(text(if is_active { "Active" } else { "Select" }))
            .padding([design::spacing::SM, design::spacing::LG])
            .style(if is_active {
                design::secondary_button
            } else {
                design::primary_button
            })
            .on_press_maybe((!is_active).then(|| Message::SelectTheme(id.to_owned())));
        let actions: Element<'a, Message> = if self.confirming_delete.as_deref() == Some(id) {
            row![
                text("Delete this theme?").color(design::TEXT_SECONDARY),
                button(text("Delete"))
                    .padding([design::spacing::SM, design::spacing::MD])
                    .style(design::danger_button)
                    .on_press(Message::DeleteTheme(id.to_owned())),
                button(text("Cancel"))
                    .padding([design::spacing::SM, design::spacing::MD])
                    .style(design::secondary_button)
                    .on_press(Message::CancelDeleteTheme),
            ]
            .spacing(design::spacing::SM)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                select,
                button(text("Preview"))
                    .padding([design::spacing::SM, design::spacing::MD])
                    .style(design::secondary_button)
                    .on_press(Message::PreviewTheme(id.to_owned())),
                button(text("Edit"))
                    .padding([design::spacing::SM, design::spacing::MD])
                    .style(design::secondary_button)
                    .on_press(Message::OpenTheme(id.to_owned())),
                button(text("Delete"))
                    .padding([design::spacing::SM, design::spacing::MD])
                    .style(design::secondary_button)
                    .on_press(Message::RequestDeleteTheme(id.to_owned())),
            ]
            .spacing(design::spacing::SM)
            .align_y(Alignment::Center)
            .into()
        };

        let description = theme
            .document
            .theme
            .description
            .as_deref()
            .unwrap_or("No description provided.");
        container(
            column![
                row![
                    column![
                        text(&theme.document.theme.name)
                            .size(design::typography::HEADING)
                            .color(design::TEXT_PRIMARY),
                        text(format!("Version {}", theme.document.theme.version))
                            .size(design::typography::CAPTION)
                            .color(design::TEXT_SECONDARY),
                    ]
                    .spacing(design::spacing::XS),
                    space::horizontal().width(Length::Fill),
                    actions,
                ]
                .align_y(Alignment::Center),
                text(description)
                    .size(design::typography::BODY)
                    .color(design::TEXT_SECONDARY),
            ]
            .spacing(design::spacing::MD),
        )
        .padding(design::spacing::LG)
        .width(Length::Fill)
        .style(|_| design::surface())
        .into()
    }

    fn invalid_theme_card<'a>(&self, issue: &'a crate::theme::ThemeIssue) -> Element<'a, Message> {
        let filename = issue
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Unknown theme file");
        container(
            column![
                text(filename)
                    .size(design::typography::HEADING)
                    .color(design::TEXT_SECONDARY),
                text(format!("Invalid theme: {}", issue.message))
                    .size(design::typography::CAPTION)
                    .color(design::TEXT_SECONDARY),
            ]
            .spacing(design::spacing::SM),
        )
        .padding(design::spacing::LG)
        .width(Length::Fill)
        .style(|_| design::surface())
        .into()
    }

    fn playback_page(&self) -> Element<'_, Message> {
        let title =
            self.playback
                .metadata
                .title
                .as_deref()
                .unwrap_or(if self.playback.is_connected() {
                    "Unknown track"
                } else {
                    "Nothing playing"
                });
        let artists = if self.playback.metadata.artists.is_empty() {
            "Unknown artist".to_owned()
        } else {
            self.playback.metadata.artists.join(" · ")
        };
        let player = self
            .playback
            .player
            .as_ref()
            .map(|player| player.identity.as_str())
            .unwrap_or("No player connected");
        let status = match self.playback.status {
            PlaybackStatus::Playing => "Playing",
            PlaybackStatus::Paused => "Paused",
            PlaybackStatus::Stopped => "Stopped",
            PlaybackStatus::Unknown => "Unknown",
        };
        let state = if self.playback.is_connected() {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };
        let source_selector: Element<'_, Message> = if self.player_selector.is_some() {
            pick_list(
                self.player_choices(),
                Some(self.selected_player.clone()),
                Message::SelectPlayer,
            )
            .placeholder("Select a player")
            .style(design::dropdown)
            .menu_style(design::dropdown_menu)
            .into()
        } else {
            text("MPRIS player discovery is unavailable.")
                .size(design::typography::BODY)
                .color(design::TEXT_SECONDARY)
                .into()
        };
        let artwork: Element<'_, Message> = if let Some(handle) = &self.artwork {
            iced::widget::image::Image::new(handle.clone())
                .width(Length::Fixed(144.0))
                .height(Length::Fixed(144.0))
                .content_fit(iced::ContentFit::Contain)
                .border_radius(design::radius::CARD)
                .into()
        } else {
            container(
                text("NO ARTWORK")
                    .size(design::typography::CAPTION)
                    .color(design::TEXT_SECONDARY),
            )
            .width(Length::Fixed(144.0))
            .height(Length::Fixed(144.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_| design::elevated_surface())
            .into()
        };

        column![
            column![
                text("Playback")
                    .size(design::typography::TITLE)
                    .color(design::TEXT_PRIMARY),
                text("Current source and track information.")
                    .size(design::typography::BODY)
                    .color(design::TEXT_SECONDARY),
            ]
            .spacing(design::spacing::XS),
            row![
                text("PLAYER SOURCE")
                    .size(design::typography::CAPTION)
                    .color(design::TEXT_SECONDARY),
                space::horizontal().width(Length::Fill),
                source_selector,
            ]
            .align_y(Alignment::Center)
            .spacing(design::spacing::MD),
            container(
                row![
                    artwork,
                    column![
                        row![
                            container(text(" "))
                                .width(8)
                                .height(8)
                                .style(move |_| design::status_indicator(state)),
                            text(if self.playback.is_connected() {
                                "CONNECTED"
                            } else {
                                "DISCONNECTED"
                            })
                            .size(design::typography::CAPTION)
                            .color(design::connection_color(state)),
                            space::horizontal().width(Length::Fill),
                            text(status)
                                .size(design::typography::CAPTION)
                                .color(design::TEXT_SECONDARY),
                        ]
                        .align_y(Alignment::Center)
                        .spacing(design::spacing::SM),
                        text(player)
                            .size(design::typography::BODY)
                            .color(design::LAVENDER),
                        text(title)
                            .size(design::typography::HEADING)
                            .color(design::TEXT_PRIMARY),
                        text(artists)
                            .size(design::typography::BODY_LARGE)
                            .color(design::LAVENDER_SOFT),
                        text(self.playback.metadata.album.as_deref().unwrap_or(""))
                            .size(design::typography::BODY)
                            .color(design::TEXT_SECONDARY),
                    ]
                    .spacing(design::spacing::MD)
                    .width(Length::Fill),
                ]
                .align_y(Alignment::Center)
                .spacing(design::spacing::XL)
            )
            .padding(design::spacing::XL)
            .width(Length::Fill)
            .style(|_| design::surface()),
        ]
        .spacing(design::spacing::XL)
        .into()
    }
}

async fn load_artwork(cache: Arc<ArtworkCache>, uri: Option<String>) -> Option<Vec<u8>> {
    let artwork = cache.resolve_or_fallback(uri.as_deref()).await;
    if artwork.mime_type == "image/svg+xml" {
        return None;
    }
    cache
        .read(&artwork.id)
        .await
        .ok()
        .flatten()
        .map(|artwork| artwork.bytes)
}

fn open_system_path(path: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let output = Command::new("xdg-open")
                    .arg(&path)
                    .output()
                    .map_err(|error| format!("could not start xdg-open: {error}"))?;
                if output.status.success() {
                    Ok(path)
                } else {
                    Err(format!(
                        "xdg-open exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ))
                }
            })
            .await
            .map_err(|error| error.to_string())?
        },
        Message::SystemPathOpened,
    )
}

async fn pick_theme_file() -> Option<PathBuf> {
    match tokio::task::spawn_blocking(pick_with_kdialog).await {
        Ok(Some(selection)) => return selection,
        Ok(None) | Err(_) => {}
    }

    rfd::AsyncFileDialog::new()
        .add_filter("Now Playing theme", &["toml"])
        .pick_file()
        .await
        .map(|file| file.path().to_owned())
}

fn pick_with_kdialog() -> Option<Option<PathBuf>> {
    let start_directory = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    let output = Command::new("kdialog")
        .arg("--getopenfilename")
        .arg(start_directory)
        .arg("Now Playing theme (*.toml)")
        .arg("Import theme")
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Some((!path.is_empty()).then(|| PathBuf::from(path)));
    }

    (output.status.code() == Some(1)).then_some(None)
}

async fn save_server_port(path: PathBuf, port: u16) -> Result<(), String> {
    let mut settings = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => toml::from_str::<toml::Value>(&contents)
            .map_err(|error| format!("Could not parse settings.toml: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(error) => return Err(format!("Could not read settings.toml: {error}")),
    };
    let Some(table) = settings.as_table_mut() else {
        return Err("The settings file must contain a TOML table.".to_owned());
    };
    table.insert(
        "server_port".to_owned(),
        toml::Value::Integer(i64::from(port)),
    );
    let contents = toml::to_string_pretty(&settings)
        .map_err(|error| format!("Could not serialize settings: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "The settings path has no parent directory.".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Could not create the settings directory: {error}"))?;
    let (temporary_path, mut file) = loop {
        let temporary_path = parent.join(format!(
            ".settings.tmp-{}-{}",
            std::process::id(),
            SETTINGS_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .await
        {
            Ok(file) => break (temporary_path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Could not create temporary settings file: {error}"));
            }
        }
    };
    if let Err(error) = file.write_all(contents.as_bytes()).await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(format!("Could not write settings: {error}"));
    }
    if let Err(error) = file.sync_all().await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(format!("Could not save settings: {error}"));
    }
    drop(file);
    if let Err(error) = tokio::fs::rename(&temporary_path, &path).await {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(format!("Could not replace settings.toml: {error}"));
    }
    Ok(())
}

fn autostart_file_path(settings_path: &std::path::Path) -> Option<PathBuf> {
    settings_path
        .parent()?
        .parent()
        .map(|config_dir| config_dir.join("autostart/now-playing-studio.desktop"))
}

async fn set_autostart(settings_path: &std::path::Path, enabled: bool) -> Result<bool, String> {
    let path = autostart_file_path(settings_path)
        .ok_or_else(|| "Could not determine the XDG autostart directory.".to_owned())?;
    if !enabled {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("Could not disable startup: {error}")),
        }
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the application executable: {error}"))?;
    let escaped_executable = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Now Playing Studio\nComment=MPRIS player monitor and OBS browser source\nExec=\"{escaped_executable}\"\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\nStartupNotify=false\n"
    );
    let parent = path
        .parent()
        .ok_or_else(|| "The XDG autostart path has no parent directory.".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Could not create the XDG autostart directory: {error}"))?;
    let temporary = parent.join(format!(".now-playing-studio.tmp-{}", std::process::id()));
    tokio::fs::write(&temporary, contents)
        .await
        .map_err(|error| format!("Could not write the startup entry: {error}"))?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|error| format!("Could not install the startup entry: {error}"))?;
    Ok(true)
}

fn app_theme(_: &App, _: window::Id) -> Theme {
    design::theme()
}

fn app_title(_: &App, _: window::Id) -> String {
    "Now Playing Studio".to_owned()
}

fn app_view(app: &App, _: window::Id) -> Element<'_, Message> {
    app.view()
}

fn app_subscription(app: &App) -> Subscription<Message> {
    app.subscription()
}

pub fn run(context: GuiContext) -> iced::Result {
    iced::daemon(move || boot(context.clone()), App::update, app_view)
        .title(app_title)
        .theme(app_theme)
        .subscription(app_subscription)
        .run()
}

fn boot(context: GuiContext) -> (App, Task<Message>) {
    let mut app = App::new(context);
    let (id, open) = window::open(window_settings());
    app.window_id = Some(id);
    (app, open.map(Message::WindowOpened))
}

fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1040.0, 680.0),
        min_size: Some(iced::Size::new(800.0, 560.0)),
        position: window::Position::Centered,
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: std::env::var("NOW_PLAYING_STUDIO_APP_ID")
                .unwrap_or_else(|_| "now-playing-studio".to_owned()),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    }
}
