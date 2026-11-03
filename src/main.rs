use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use fs2::FileExt;
use nowplaying::{
    artwork::{ArtworkCache, ArtworkCacheConfig},
    gui::app::{self as gui, GuiContext},
    mpris::{PlaybackHandle, PlayerSelection},
    runtime::PlaybackApi,
    server::LocalServer,
    theme::{ThemeLoader, ThemeManager},
    tray::{self, TrayCommand},
};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_THEME_ID: &str = "catppuccin-mocha";
const DEFAULT_SERVER_PORT: u16 = 9005;

struct AppPaths {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
}

const LEGACY_APP_DIRECTORY: &str = "now-playing-studio";

struct RunningApp {
    themes: ThemeManager,
    discovery: nowplaying::theme::ThemeDiscovery,
    active_theme: Option<String>,
    family_selections: BTreeMap<String, String>,
    playback_provider: Option<PlaybackHandle>,
    playback_api: Arc<PlaybackApi>,
    artwork_cache: Arc<ArtworkCache>,
    server: LocalServer,
    settings_path: PathBuf,
    themes_dir: PathBuf,
}

impl AppPaths {
    fn discover() -> io::Result<Self> {
        Ok(Self {
            config: xdg_path("XDG_CONFIG_HOME", ".config")?.join("nowplaying"),
            data: xdg_path("XDG_DATA_HOME", ".local/share")?.join("nowplaying"),
            cache: xdg_path("XDG_CACHE_HOME", ".cache")?.join("nowplaying"),
        })
    }

    fn instance_lock_path(&self) -> PathBuf {
        let legacy_config = legacy_sibling(&self.config);
        if legacy_config.exists() {
            legacy_config.join("instance.lock")
        } else {
            self.config.join("instance.lock")
        }
    }

    async fn migrate_legacy_directories(&self) -> io::Result<()> {
        migrate_directory(&legacy_sibling(&self.config), &self.config).await?;
        migrate_directory(&legacy_sibling(&self.data), &self.data).await?;
        migrate_directory(&legacy_sibling(&self.cache), &self.cache).await
    }
}

fn legacy_sibling(path: &std::path::Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(LEGACY_APP_DIRECTORY)
}

async fn migrate_directory(legacy: &std::path::Path, current: &std::path::Path) -> io::Result<()> {
    if !tokio::fs::try_exists(legacy).await? {
        return Ok(());
    }
    if !tokio::fs::try_exists(current).await? {
        return tokio::fs::rename(legacy, current).await;
    }

    tokio::fs::create_dir_all(current).await?;
    let mut entries = tokio::fs::read_dir(legacy).await?;
    let mut has_unmigrated_entries = false;
    while let Some(entry) = entries.next_entry().await? {
        let source = entry.path();
        let destination = current.join(entry.file_name());
        if !tokio::fs::try_exists(&destination).await? {
            tokio::fs::rename(source, destination).await?;
            continue;
        }

        let source_is_directory = entry.file_type().await?.is_dir();
        let destination_is_directory = tokio::fs::metadata(&destination).await?.is_dir();
        if source_is_directory && destination_is_directory {
            Box::pin(migrate_directory(&source, &destination)).await?;
            if tokio::fs::try_exists(&source).await? {
                has_unmigrated_entries = true;
            }
        } else {
            warn!(path = %source.display(), "Keeping a legacy app file because its new-name path already exists");
            has_unmigrated_entries = true;
        }
    }

    if !has_unmigrated_entries {
        tokio::fs::remove_dir(legacy).await?;
    }
    Ok(())
}

fn xdg_path(variable: &str, home_suffix: &str) -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(home.join(home_suffix))
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let paths = AppPaths::discover()?;
    let Some(_instance_lock) = InstanceLock::acquire(&paths.instance_lock_path())? else {
        info!("nowplaying is already running");
        return Ok(());
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut app = runtime.block_on(start_runtime())?;
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let restart_requested = Arc::new(AtomicBool::new(false));
    let tray_available = Arc::new(AtomicBool::new(true));
    let show_requested = Arc::new(AtomicBool::new(false));
    let (tray_sender, mut tray_receiver) = tokio::sync::mpsc::unbounded_channel();
    let tray_handle = match runtime.block_on(tray::start(tray_sender, Arc::clone(&tray_available)))
    {
        Ok(handle) => {
            info!("StatusNotifierItem tray started");
            Some(handle)
        }
        Err(error) => {
            tray_available.store(false, Ordering::Relaxed);
            warn!(%error, "System tray is unavailable; window close will exit the app");
            None
        }
    };
    let show_from_tray = Arc::clone(&show_requested);
    let quit_from_tray = Arc::clone(&shutdown_requested);
    runtime.handle().spawn(async move {
        while let Some(command) = tray_receiver.recv().await {
            match command {
                TrayCommand::Show => show_from_tray.store(true, Ordering::Relaxed),
                TrayCommand::Quit => quit_from_tray.store(true, Ordering::Relaxed),
            }
        }
    });
    let signal = Arc::clone(&shutdown_requested);
    runtime.handle().spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal.store(true, Ordering::Relaxed);
        }
    });

    let gui_result = gui::run(GuiContext {
        themes: app.themes.clone(),
        catalog: app.discovery.clone(),
        active_theme: app.active_theme.clone(),
        family_selections: app.family_selections.clone(),
        playback_state: app.playback_provider.as_ref().map(PlaybackHandle::state),
        players_state: app
            .playback_provider
            .as_ref()
            .map(PlaybackHandle::available_players),
        player_selector: app.playback_provider.as_ref().map(PlaybackHandle::selector),
        artwork_cache: Arc::clone(&app.artwork_cache),
        service_url: format!("http://{}/", app.server.address()),
        settings_path: app.settings_path.clone(),
        themes_dir: app.themes_dir.clone(),
        restart_requested: Arc::clone(&restart_requested),
        tray_available: Arc::clone(&tray_available),
        show_requested,
        shutdown_requested: Arc::clone(&shutdown_requested),
    });

    if let Some(tray) = tray_handle {
        runtime.block_on(tray.shutdown());
    }
    let server_result = runtime.block_on(app.server.shutdown());
    drop(app.playback_api);
    if let Some(provider) = app.playback_provider.take() {
        runtime.block_on(provider.shutdown());
    }
    gui_result?;
    server_result?;
    // Release the single-instance lock before spawning a replacement process.
    // Otherwise the new process can see this lock and exit during startup.
    drop(_instance_lock);
    if restart_requested.load(Ordering::Relaxed) {
        Command::new(std::env::current_exe()?).spawn()?;
    }
    Ok(())
}

struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    fn acquire(path: &std::path::Path) -> io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

async fn start_runtime() -> Result<RunningApp, Box<dyn Error>> {
    let paths = AppPaths::discover()?;
    paths.migrate_legacy_directories().await?;
    tokio::fs::create_dir_all(&paths.config).await?;
    tokio::fs::create_dir_all(&paths.data).await?;
    tokio::fs::create_dir_all(&paths.cache).await?;

    let themes_dir = paths.data.join("themes");
    tokio::fs::create_dir_all(&themes_dir).await?;
    install_bundled_themes(&themes_dir).await?;

    let settings_path = paths.config.join("settings.toml");
    if let Err(error) = gui::migrate_legacy_autostart(&settings_path).await {
        warn!(%error, "Could not migrate the old autostart entry");
    }
    let themes = ThemeManager::new(&themes_dir, &settings_path, ThemeLoader::current()).await?;
    let discovery = themes.refresh().await?;
    for issue in &discovery.issues {
        warn!(path = %issue.path.display(), message = %issue.message, "Theme was skipped");
    }
    restore_startup_theme(&themes).await?;
    let active_theme = themes.active().await.map(|theme| theme.id().to_owned());
    let family_selections = themes.selected_family_variants().await;

    let artwork = Arc::new(
        ArtworkCache::new(paths.cache.join("artwork"), ArtworkCacheConfig::default()).await?,
    );
    let (playback_provider, playback_api) =
        match PlaybackHandle::connect(PlayerSelection::Automatic).await {
            Ok(provider) => {
                info!("MPRIS playback monitoring started");
                let api = Arc::new(PlaybackApi::start(&provider));
                (Some(provider), api)
            }
            Err(error) => {
                error!(%error, "MPRIS is unavailable; starting with disconnected playback state");
                (None, Arc::new(PlaybackApi::disconnected()))
            }
        };

    let server_address = configured_server_address(&settings_path).await;
    let server = match LocalServer::start(
        server_address,
        playback_api.clone(),
        themes.clone(),
        Arc::clone(&artwork),
    )
    .await
    {
        Ok(server) => server,
        Err(error) => {
            if let Some(provider) = playback_provider {
                provider.shutdown().await;
            }
            return Err(Box::<dyn Error>::from(error));
        }
    };
    info!(address = %server.address(), "OBS Browser Source server started");

    Ok(RunningApp {
        themes,
        discovery,
        active_theme,
        family_selections,
        playback_provider,
        playback_api,
        artwork_cache: artwork,
        server,
        settings_path,
        themes_dir,
    })
}

async fn install_bundled_themes(themes_dir: &std::path::Path) -> io::Result<()> {
    for (filename, contents) in [
        (
            "catppuccin-mocha.toml",
            include_bytes!("../themes/catppuccin-mocha.toml").as_slice(),
        ),
        (
            "quiet-mocha.toml",
            include_bytes!("../themes/quiet-mocha.toml").as_slice(),
        ),
        (
            "starfall.toml",
            include_bytes!("../themes/starfall.toml").as_slice(),
        ),
        (
            "spectral-tide.toml",
            include_bytes!("../themes/spectral-tide.toml").as_slice(),
        ),
    ] {
        install_bundled_theme(themes_dir, filename, contents).await?;
    }
    Ok(())
}

async fn install_bundled_theme(
    themes_dir: &std::path::Path,
    filename: &str,
    contents: &[u8],
) -> io::Result<()> {
    let destination = themes_dir.join(filename);
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .await
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error),
    };
    if let Err(error) = file.write_all(contents).await {
        drop(file);
        let _ = tokio::fs::remove_file(destination).await;
        return Err(error);
    }
    if let Err(error) = file.sync_all().await {
        drop(file);
        let _ = tokio::fs::remove_file(destination).await;
        return Err(error);
    }
    Ok(())
}

async fn restore_startup_theme(themes: &ThemeManager) -> Result<(), Box<dyn Error>> {
    match themes.restore_selection().await {
        Ok(Some(theme)) => {
            info!(theme_id = theme.id(), "Restored selected theme");
            return Ok(());
        }
        Ok(None) => info!("No theme is selected; choosing the reference theme"),
        Err(error) => warn!(%error, "Could not restore the selected theme"),
    }

    match themes.select(DEFAULT_THEME_ID).await {
        Ok(theme) => info!(theme_id = theme.id(), "Selected the reference theme"),
        Err(error) => {
            warn!(%error, "Could not persist the default theme selection; activating it for this run");
            themes.activate(DEFAULT_THEME_ID).await?;
        }
    }
    Ok(())
}

async fn configured_server_address(settings_path: &std::path::Path) -> SocketAddr {
    let default = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_SERVER_PORT);
    let settings = match tokio::fs::read_to_string(settings_path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return default,
        Err(error) => {
            warn!(%error, "Could not read settings for the server port; using the default");
            return default;
        }
    };
    let value: toml::Value = match toml::from_str(&settings) {
        Ok(value) => value,
        Err(error) => {
            warn!(%error, "Settings are invalid; using the default server port");
            return default;
        }
    };
    let port = value
        .get("server_port")
        .and_then(toml::Value::as_integer)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0);
    match port {
        Some(port) => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        None => default,
    }
}

#[cfg(test)]
mod bundled_theme_tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::install_bundled_themes;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn installs_all_bundled_themes_without_replacing_existing_files() {
        let directory = std::env::temp_dir().join(format!(
            "nowplaying-bundled-themes-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let quiet_theme = directory.join("quiet-mocha.toml");
        fs::write(&quiet_theme, "user-owned theme contents").unwrap();

        install_bundled_themes(&directory).await.unwrap();

        for filename in [
            "catppuccin-mocha.toml",
            "quiet-mocha.toml",
            "starfall.toml",
            "spectral-tide.toml",
        ] {
            assert!(directory.join(filename).is_file(), "missing {filename}");
        }
        assert_eq!(
            fs::read_to_string(quiet_theme).unwrap(),
            "user-owned theme contents"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
