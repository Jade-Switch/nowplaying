use std::{
    collections::{BTreeMap, btree_map::Entry},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{RwLock, watch},
};
use tracing::warn;

pub const THEME_FORMAT_VERSION: u32 = 1;
pub const MAX_THEME_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_THEME_ASSET_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_THEME_ASSETS: usize = 64;
pub const MAX_THEME_ASSET_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_SETTINGS_BYTES: usize = 1024 * 1024;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme file is too large")]
    TooLarge,
    #[error("theme file is not valid UTF-8")]
    InvalidUtf8,
    #[error("theme TOML is invalid: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("theme manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("unsupported theme format version {0}")]
    UnsupportedFormat(u32),
    #[error("theme requires runtime {required}, current runtime is {current}")]
    RuntimeTooOld { required: String, current: String },
    #[error("theme {0} was not found")]
    NotFound(String),
    #[error("cannot delete the active theme; select a different theme first")]
    ActiveTheme,
    #[error("a theme with ID {0} already exists")]
    AlreadyExists(String),
    #[error("theme asset was not found: {0}")]
    AssetNotFound(String),
    #[error("theme settings file is invalid")]
    InvalidSettings,
    #[error("theme filesystem operation failed")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeDocument {
    pub theme: ThemeIdentity,
    #[serde(default)]
    pub canvas: CanvasConfig,
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub artwork: ArtworkConfig,
    #[serde(default)]
    pub metadata: MetadataConfig,
    #[serde(default)]
    pub progress: ProgressConfig,
    #[serde(default)]
    pub animations: AnimationConfig,
    #[serde(default)]
    pub colors: ColorConfig,
    pub html: HtmlContent,
    #[serde(default)]
    pub css: CssContent,
    #[serde(default)]
    pub javascript: JavaScriptContent,
    #[serde(default)]
    pub assets: BTreeMap<String, AssetDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    pub format_version: u32,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub compatibility: ThemeCompatibility,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeCompatibility {
    pub minimum_runtime: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CanvasConfig {
    pub width: u32,
    pub height: u32,
    pub background: String,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 180,
            background: "transparent".into(),
        }
    }
}

impl CanvasConfig {
    /// Returns an official built-in size profile for exact standard dimensions.
    /// Custom dimensions remain valid and have no matching profile.
    pub fn profile_name(&self) -> Option<&'static str> {
        match (self.width, self.height) {
            (420, 64) => Some("Slim"),
            (420, 120) => Some("Compact"),
            (800, 150) => Some("Standard"),
            (960, 150) => Some("Wide"),
            (1000, 180) => Some("Ultra-wide"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
    #[default]
    Freeform,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    Start,
    End,
    #[default]
    Center,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutConfig {
    pub direction: LayoutDirection,
    pub alignment: Alignment,
    pub spacing: u16,
    pub padding: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            direction: LayoutDirection::Freeform,
            alignment: Alignment::Center,
            spacing: 12,
            padding: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkPosition {
    #[default]
    Hidden,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArtworkConfig {
    pub enabled: bool,
    pub position: ArtworkPosition,
    pub width: u16,
    pub height: u16,
    pub fit: String,
    pub rendering: String,
}

impl Default for ArtworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            position: ArtworkPosition::Hidden,
            width: 0,
            height: 0,
            fit: "cover".into(),
            rendering: "auto".into(),
        }
    }
}

impl ArtworkConfig {
    pub fn should_load(&self) -> bool {
        self.enabled && self.width > 0 && self.height > 0
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetadataConfig {
    pub show_title: bool,
    pub show_artist: bool,
    pub show_album: bool,
    pub alignment: Alignment,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            show_title: true,
            show_artist: true,
            show_album: false,
            alignment: Alignment::Start,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProgressConfig {
    pub enabled: bool,
    pub position: String,
    pub height: u16,
}

impl Default for ProgressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            position: "bottom".into(),
            height: 4,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationConfig {
    pub enabled: bool,
    pub reduced_motion_support: bool,
    pub track_change: String,
    pub track_change_duration_ms: u16,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reduced_motion_support: true,
            track_change: "none".into(),
            track_change_duration_ms: 450,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorConfig {
    pub background: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            background: "transparent".into(),
            text: "#ffffff".into(),
            muted: "#aaaaaa".into(),
            accent: "#cba6f7".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HtmlContent {
    pub template: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CssContent {
    pub stylesheet: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavaScriptContent {
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetDefinition {
    pub mime_type: String,
    /// RFC 4648 standard Base64. The manifest declares the MIME type explicitly.
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct ThemeAsset {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LoadedTheme {
    pub document: ThemeDocument,
    pub assets: BTreeMap<String, ThemeAsset>,
    pub source_path: Option<PathBuf>,
}

impl LoadedTheme {
    pub fn id(&self) -> &str {
        &self.document.theme.id
    }

    pub fn asset(&self, id: &str) -> Result<&ThemeAsset, ThemeError> {
        self.assets
            .get(id)
            .ok_or_else(|| ThemeError::AssetNotFound(id.to_owned()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ThemeDiscovery {
    pub themes: BTreeMap<String, Arc<LoadedTheme>>,
    pub issues: Vec<ThemeIssue>,
}

#[derive(Debug, Clone)]
pub struct ThemeIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ThemeLoader {
    runtime_version: Version,
}

impl ThemeLoader {
    pub fn new(runtime_version: &str) -> Result<Self, ThemeError> {
        let runtime_version = Version::parse(runtime_version)
            .map_err(|_| ThemeError::InvalidManifest("invalid runtime version".into()))?;
        Ok(Self { runtime_version })
    }

    pub fn current() -> Self {
        Self::new(env!("CARGO_PKG_VERSION")).expect("package version is valid SemVer")
    }

    pub fn parse(&self, bytes: &[u8]) -> Result<LoadedTheme, ThemeError> {
        if bytes.len() > MAX_THEME_FILE_BYTES {
            return Err(ThemeError::TooLarge);
        }
        let source = std::str::from_utf8(bytes).map_err(|_| ThemeError::InvalidUtf8)?;
        let document: ThemeDocument = toml::from_str(source)?;
        self.validate(document)
    }

    pub async fn load_file(&self, path: impl AsRef<Path>) -> Result<LoadedTheme, ThemeError> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path).await?;
        if !metadata.is_file() {
            return Err(ThemeError::InvalidManifest(
                "theme path is not a regular file".into(),
            ));
        }
        if metadata.len() > MAX_THEME_FILE_BYTES as u64 {
            return Err(ThemeError::TooLarge);
        }
        let bytes = read_limited(path, MAX_THEME_FILE_BYTES).await?;
        let mut theme = self.parse(&bytes)?;
        theme.source_path = Some(path.to_owned());
        Ok(theme)
    }

    pub async fn discover(
        &self,
        directory: impl AsRef<Path>,
    ) -> Result<ThemeDiscovery, ThemeError> {
        let mut read_dir = tokio::fs::read_dir(directory.as_ref()).await?;
        let mut paths = Vec::new();
        while let Some(entry) = read_dir.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "toml")
            {
                paths.push(entry.path());
            }
        }
        paths.sort();

        let mut discovery = ThemeDiscovery::default();
        for path in paths {
            match self.load_file(&path).await {
                Ok(theme) => {
                    let id = theme.id().to_owned();
                    match discovery.themes.entry(id.clone()) {
                        Entry::Vacant(entry) => {
                            entry.insert(Arc::new(theme));
                        }
                        Entry::Occupied(_) => discovery.issues.push(ThemeIssue {
                            path,
                            message: format!("duplicate theme ID: {id}"),
                        }),
                    }
                }
                Err(error) => {
                    warn!(path = %path.display(), %error, "Skipping invalid theme");
                    discovery.issues.push(ThemeIssue {
                        path,
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(discovery)
    }

    fn validate(&self, document: ThemeDocument) -> Result<LoadedTheme, ThemeError> {
        validate_document(&document, &self.runtime_version)?;
        let mut assets = BTreeMap::new();
        let mut total_asset_bytes = 0_usize;
        for (id, definition) in &document.assets {
            let max_encoded = MAX_THEME_ASSET_BYTES.div_ceil(3) * 4;
            if definition.data.len() > max_encoded {
                return Err(ThemeError::TooLarge);
            }
            let bytes = BASE64.decode(definition.data.as_bytes()).map_err(|_| {
                ThemeError::InvalidManifest(format!("asset {id} is invalid Base64"))
            })?;
            if bytes.is_empty() || bytes.len() > MAX_THEME_ASSET_BYTES {
                return Err(if bytes.is_empty() {
                    ThemeError::InvalidManifest(format!("asset {id} is empty"))
                } else {
                    ThemeError::TooLarge
                });
            }
            total_asset_bytes = total_asset_bytes.saturating_add(bytes.len());
            if total_asset_bytes > MAX_THEME_ASSET_TOTAL_BYTES {
                return Err(ThemeError::TooLarge);
            }
            assets.insert(
                id.clone(),
                ThemeAsset {
                    mime_type: definition.mime_type.clone(),
                    bytes,
                },
            );
        }
        Ok(LoadedTheme {
            document,
            assets,
            source_path: None,
        })
    }
}

fn validate_document(
    document: &ThemeDocument,
    runtime_version: &Version,
) -> Result<(), ThemeError> {
    let identity = &document.theme;
    if !valid_theme_id(&identity.id) {
        return invalid("theme.id must use lowercase letters, digits and hyphens");
    }
    validate_text("theme.name", &identity.name, 128, true)?;
    if let Some(author) = &identity.author {
        validate_text("theme.author", author, 128, false)?;
    }
    if let Some(description) = &identity.description {
        validate_text("theme.description", description, 1024, false)?;
    }
    Version::parse(&identity.version).map_err(|_| {
        ThemeError::InvalidManifest("theme.version must be semantic versioning".into())
    })?;
    if identity.format_version != THEME_FORMAT_VERSION {
        return Err(ThemeError::UnsupportedFormat(identity.format_version));
    }
    let minimum = Version::parse(&identity.compatibility.minimum_runtime).map_err(|_| {
        ThemeError::InvalidManifest(
            "theme.compatibility.minimum_runtime must be semantic versioning".into(),
        )
    })?;
    if minimum > *runtime_version {
        return Err(ThemeError::RuntimeTooOld {
            required: minimum.to_string(),
            current: runtime_version.to_string(),
        });
    }
    if !(1..=4096).contains(&document.canvas.width) || !(1..=4096).contains(&document.canvas.height)
    {
        return invalid("canvas width and height must be between 1 and 4096");
    }
    validate_text("canvas.background", &document.canvas.background, 256, true)?;
    if document.layout.spacing > 256 || document.layout.padding > 256 {
        return invalid("layout spacing and padding must be at most 256");
    }
    if document.artwork.width > 2048 || document.artwork.height > 2048 {
        return invalid("artwork width and height must be at most 2048");
    }
    validate_text("artwork.fit", &document.artwork.fit, 32, true)?;
    validate_text("artwork.rendering", &document.artwork.rendering, 32, true)?;
    validate_text("progress.position", &document.progress.position, 32, true)?;
    if !(1..=64).contains(&document.progress.height) {
        return invalid("progress height must be between 1 and 64");
    }
    validate_text(
        "animations.track_change",
        &document.animations.track_change,
        64,
        true,
    )?;
    if document.animations.track_change_duration_ms > 10_000 {
        return invalid("track change duration must be at most 10000ms");
    }
    for (name, color) in [
        ("colors.background", &document.colors.background),
        ("colors.text", &document.colors.text),
        ("colors.muted", &document.colors.muted),
        ("colors.accent", &document.colors.accent),
    ] {
        validate_text(name, color, 128, true)?;
    }
    if document.html.template.trim().is_empty() {
        return invalid("html.template must not be empty");
    }
    if document.html.template.len() > 4 * 1024 * 1024
        || document.css.stylesheet.len() > 1024 * 1024
        || document.javascript.source.len() > 1024 * 1024
    {
        return Err(ThemeError::TooLarge);
    }
    if document.assets.len() > MAX_THEME_ASSETS {
        return Err(ThemeError::TooLarge);
    }
    for (id, asset) in &document.assets {
        if !valid_asset_id(id) {
            return invalid("asset IDs must use lowercase letters, digits, hyphens or underscores");
        }
        if !valid_mime_type(&asset.mime_type) {
            return invalid("each embedded asset needs a valid explicit MIME type");
        }
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, ThemeError> {
    Err(ThemeError::InvalidManifest(message.to_owned()))
}

fn validate_text(
    field: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), ThemeError> {
    if (!allow_empty && value.trim().is_empty())
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
    {
        return invalid(&format!(
            "{field} is empty, too long or contains control characters"
        ));
    }
    Ok(())
}

fn valid_theme_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_asset_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn valid_mime_type(value: &str) -> bool {
    if value.len() > 127 {
        return false;
    }
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    let valid_token = |token: &str| {
        !token.is_empty()
            && token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    valid_token(kind) && valid_token(subtype)
}

#[derive(Debug, Clone)]
pub struct ThemeManager {
    themes_dir: PathBuf,
    settings_path: PathBuf,
    loader: ThemeLoader,
    catalog: Arc<RwLock<ThemeDiscovery>>,
    active: Arc<RwLock<Option<Arc<LoadedTheme>>>>,
    mutation: Arc<tokio::sync::Mutex<()>>,
    theme_revision: watch::Sender<u64>,
}

impl ThemeManager {
    pub async fn new(
        themes_dir: impl AsRef<Path>,
        settings_path: impl AsRef<Path>,
        loader: ThemeLoader,
    ) -> Result<Self, ThemeError> {
        tokio::fs::create_dir_all(themes_dir.as_ref()).await?;
        let (theme_revision, _) = watch::channel(0);
        Ok(Self {
            themes_dir: tokio::fs::canonicalize(themes_dir.as_ref()).await?,
            settings_path: settings_path.as_ref().to_owned(),
            loader,
            catalog: Arc::new(RwLock::new(ThemeDiscovery::default())),
            active: Arc::new(RwLock::new(None)),
            mutation: Arc::new(tokio::sync::Mutex::new(())),
            theme_revision,
        })
    }

    pub async fn refresh(&self) -> Result<ThemeDiscovery, ThemeError> {
        let discovery = self.loader.discover(&self.themes_dir).await?;
        *self.catalog.write().await = discovery.clone();
        Ok(discovery)
    }

    pub async fn catalog(&self) -> ThemeDiscovery {
        self.catalog.read().await.clone()
    }

    pub async fn active(&self) -> Option<Arc<LoadedTheme>> {
        self.active.read().await.clone()
    }

    /// Subscribe to successful theme activations for connected browser clients.
    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.theme_revision.subscribe()
    }

    fn notify_theme_change(&self) {
        self.theme_revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Activate an already validated catalog entry without changing settings.
    /// This is useful for startup recovery when persisted settings are malformed.
    pub async fn activate(&self, id: &str) -> Result<Arc<LoadedTheme>, ThemeError> {
        let _mutation = self.mutation.lock().await;
        let theme = self
            .catalog
            .read()
            .await
            .themes
            .get(id)
            .cloned()
            .ok_or_else(|| ThemeError::NotFound(id.to_owned()))?;
        *self.active.write().await = Some(theme.clone());
        self.notify_theme_change();
        Ok(theme)
    }

    /// Select only a fully parsed and validated theme. Persist first, then
    /// replace the active theme so a failed switch leaves the old theme active.
    pub async fn select(&self, id: &str) -> Result<Arc<LoadedTheme>, ThemeError> {
        let _mutation = self.mutation.lock().await;
        let theme = self
            .catalog
            .read()
            .await
            .themes
            .get(id)
            .cloned()
            .ok_or_else(|| ThemeError::NotFound(id.to_owned()))?;
        save_selected_id(&self.settings_path, id).await?;
        *self.active.write().await = Some(theme.clone());
        self.notify_theme_change();
        Ok(theme)
    }

    pub async fn delete(&self, id: &str) -> Result<(), ThemeError> {
        let _mutation = self.mutation.lock().await;
        if self
            .active
            .read()
            .await
            .as_ref()
            .is_some_and(|theme| theme.id() == id)
        {
            return Err(ThemeError::ActiveTheme);
        }
        let path = self
            .catalog
            .read()
            .await
            .themes
            .get(id)
            .and_then(|theme| theme.source_path.clone())
            .ok_or_else(|| ThemeError::NotFound(id.to_owned()))?;
        tokio::fs::remove_file(path).await?;
        self.catalog.write().await.themes.remove(id);
        Ok(())
    }

    pub async fn restore_selection(&self) -> Result<Option<Arc<LoadedTheme>>, ThemeError> {
        let Some(id) = read_selected_id(&self.settings_path).await? else {
            return Ok(None);
        };
        self.select(&id).await.map(Some)
    }

    pub async fn import_file(
        &self,
        source: impl AsRef<Path>,
    ) -> Result<Arc<LoadedTheme>, ThemeError> {
        let source = source.as_ref();
        let metadata = tokio::fs::metadata(source).await?;
        if metadata.len() > MAX_THEME_FILE_BYTES as u64 {
            return Err(ThemeError::TooLarge);
        }
        if !metadata.is_file() {
            return Err(ThemeError::InvalidManifest(
                "import source is not a file".into(),
            ));
        }
        let bytes = read_limited(source, MAX_THEME_FILE_BYTES).await?;
        self.import_bytes(&bytes).await
    }

    /// Parse and store theme text using the same validation and duplicate-safe
    /// persistence path as file imports.
    pub async fn import_content(&self, source: &str) -> Result<Arc<LoadedTheme>, ThemeError> {
        self.import_bytes(source.as_bytes()).await
    }

    async fn import_bytes(&self, bytes: &[u8]) -> Result<Arc<LoadedTheme>, ThemeError> {
        if bytes.len() > MAX_THEME_FILE_BYTES {
            return Err(ThemeError::TooLarge);
        }
        let mut theme = self.loader.parse(bytes)?;
        let id = theme.id().to_owned();
        let destination = self.themes_dir.join(format!("{id}.toml"));
        atomic_create_new(&destination, bytes, &id).await?;
        theme.source_path = Some(destination);
        let theme = Arc::new(theme);
        self.catalog.write().await.themes.insert(id, theme.clone());
        Ok(theme)
    }

    pub async fn export_file(
        &self,
        id: &str,
        destination: impl AsRef<Path>,
    ) -> Result<(), ThemeError> {
        let theme = self
            .catalog
            .read()
            .await
            .themes
            .get(id)
            .cloned()
            .ok_or_else(|| ThemeError::NotFound(id.to_owned()))?;
        let source = theme
            .source_path
            .as_ref()
            .ok_or_else(|| ThemeError::NotFound(id.to_owned()))?;
        let metadata = tokio::fs::metadata(source).await?;
        if metadata.len() > MAX_THEME_FILE_BYTES as u64 {
            return Err(ThemeError::TooLarge);
        }
        let bytes = read_limited(source, MAX_THEME_FILE_BYTES).await?;
        let verified = self.loader.parse(&bytes)?;
        if verified.id() != id {
            return Err(ThemeError::InvalidManifest(
                "theme file changed after discovery".into(),
            ));
        }
        atomic_create_new(destination.as_ref(), &bytes, id).await
    }
}

pub async fn read_selected_id(
    settings_path: impl AsRef<Path>,
) -> Result<Option<String>, ThemeError> {
    let bytes = match read_limited(settings_path.as_ref(), MAX_SETTINGS_BYTES).await {
        Ok(bytes) => bytes,
        Err(ThemeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(ThemeError::TooLarge) => return Err(ThemeError::InvalidSettings),
        Err(error) => return Err(error),
    };
    let value: toml::Value = toml::from_slice(&bytes).map_err(|_| ThemeError::InvalidSettings)?;
    let id = value
        .as_table()
        .and_then(|table| table.get("selected_theme_id"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if id.as_deref().is_some_and(|id| !valid_theme_id(id)) {
        return Err(ThemeError::InvalidSettings);
    }
    Ok(id)
}

async fn save_selected_id(path: &Path, id: &str) -> Result<(), ThemeError> {
    if !valid_theme_id(id) {
        return Err(ThemeError::InvalidSettings);
    }
    let mut settings = match read_limited(path, MAX_SETTINGS_BYTES).await {
        Ok(bytes) => {
            toml::from_slice::<toml::Value>(&bytes).map_err(|_| ThemeError::InvalidSettings)?
        }
        Err(ThemeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(error) => return Err(error),
    };
    let table = settings.as_table_mut().ok_or(ThemeError::InvalidSettings)?;
    table.insert(
        "selected_theme_id".into(),
        toml::Value::String(id.to_owned()),
    );
    let serialized = toml::to_string_pretty(&settings).map_err(|_| ThemeError::InvalidSettings)?;
    if serialized.len() > MAX_SETTINGS_BYTES {
        return Err(ThemeError::InvalidSettings);
    }
    atomic_replace(path, serialized.as_bytes(), "settings").await?;
    Ok(())
}

async fn atomic_create_new(path: &Path, bytes: &[u8], label: &str) -> Result<(), ThemeError> {
    let parent = path.parent().ok_or_else(|| {
        ThemeError::InvalidManifest("destination must have a parent directory".into())
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let temp_path = temp_path(parent, label);
    write_temp(&temp_path, bytes).await?;
    match tokio::fs::hard_link(&temp_path, path).await {
        Ok(()) => {
            tokio::fs::remove_file(temp_path).await?;
            Ok(())
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Err(ThemeError::AlreadyExists(label.to_owned()))
            } else {
                Err(error.into())
            }
        }
    }
}

async fn atomic_replace(path: &Path, bytes: &[u8], label: &str) -> Result<(), ThemeError> {
    let parent = path.parent().ok_or(ThemeError::InvalidSettings)?;
    tokio::fs::create_dir_all(parent).await?;
    let temp_path = temp_path(parent, label);
    write_temp(&temp_path, bytes).await?;
    if let Err(error) = tokio::fs::rename(&temp_path, path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error.into());
    }
    Ok(())
}

fn temp_path(parent: &Path, label: &str) -> PathBuf {
    parent.join(format!(
        ".{label}.tmp-{}-{}",
        std::process::id(),
        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

async fn write_temp(path: &Path, bytes: &[u8]) -> Result<(), ThemeError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await?;
    if let Err(error) = file.write_all(bytes).await {
        let _ = tokio::fs::remove_file(path).await;
        return Err(error.into());
    }
    if let Err(error) = file.sync_all().await {
        drop(file);
        let _ = tokio::fs::remove_file(path).await;
        return Err(error.into());
    }
    Ok(())
}

async fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, ThemeError> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        return Err(ThemeError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_THEME: &str = r#"
[theme]
id = "sample-player"
name = "Sample Player"
version = "1.0.0"
format_version = 1
author = "Test Author"
description = "A test fixture"

[theme.compatibility]
minimum_runtime = "1.0.0"

[html]
template = "<main id='player'></main>"
"#;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "nps-theme-{label}-{}-{}",
                std::process::id(),
                TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn loader() -> ThemeLoader {
        ThemeLoader::new("1.2.0").unwrap()
    }

    #[test]
    fn valid_theme_loads_with_defaults_and_decodes_embedded_resources() {
        let source = format!(
            "{VALID_THEME}\n[assets.brand_mark]\nmime_type = \"image/gif\"\ndata = \"R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=\"\n"
        );
        let theme = loader().parse(source.as_bytes()).unwrap();
        assert_eq!(theme.id(), "sample-player");
        assert_eq!(theme.document.canvas.width, 640);
        assert_eq!(theme.document.canvas.height, 180);
        assert_eq!(theme.document.artwork.position, ArtworkPosition::Hidden);
        assert!(!theme.document.progress.enabled);
        assert_eq!(theme.assets["brand_mark"].mime_type, "image/gif");
        assert!(!theme.assets["brand_mark"].bytes.is_empty());
    }

    #[test]
    fn bundled_reference_theme_is_valid_for_the_current_runtime() {
        let theme = loader()
            .parse(include_bytes!("../themes/catppuccin-mocha.toml"))
            .unwrap();
        assert_eq!(theme.id(), "catppuccin-mocha");
        assert_eq!(theme.document.canvas.width, 800);
        assert_eq!(theme.document.canvas.height, 150);
        assert!(
            theme
                .document
                .javascript
                .source
                .contains("NowPlaying.subscribe")
        );
        assert!(
            theme
                .document
                .css
                .stylesheet
                .contains("prefers-reduced-motion")
        );
    }

    #[test]
    fn all_bundled_themes_parse_and_validate_for_the_current_runtime() {
        for (expected_id, source, expected_dimensions, expected_profile) in [
            (
                "catppuccin-mocha",
                include_bytes!("../themes/catppuccin-mocha.toml").as_slice(),
                (800, 150),
                "Standard",
            ),
            (
                "quiet-mocha",
                include_bytes!("../themes/quiet-mocha.toml").as_slice(),
                (420, 64),
                "Slim",
            ),
            (
                "starfall",
                include_bytes!("../themes/starfall.toml").as_slice(),
                (1000, 180),
                "Ultra-wide",
            ),
            (
                "spectral-tide",
                include_bytes!("../themes/spectral-tide.toml").as_slice(),
                (1000, 180),
                "Ultra-wide",
            ),
        ] {
            let theme = loader().parse(source).unwrap_or_else(|error| {
                panic!("bundled theme {expected_id} failed validation: {error}")
            });
            assert_eq!(theme.id(), expected_id);
            assert_eq!(
                (theme.document.canvas.width, theme.document.canvas.height),
                expected_dimensions
            );
            assert!(
                theme
                    .document
                    .css
                    .stylesheet
                    .contains(&format!("width: {}px;", expected_dimensions.0))
            );
            assert!(
                theme
                    .document
                    .css
                    .stylesheet
                    .contains(&format!("height: {}px;", expected_dimensions.1))
            );
            assert_eq!(theme.document.canvas.profile_name(), Some(expected_profile));
        }
    }

    #[test]
    fn custom_canvas_sizes_remain_valid_and_unprofiled() {
        let source = format!("[canvas]\nwidth = 1234\nheight = 567\n\n{VALID_THEME}");
        let theme = loader().parse(source.as_bytes()).unwrap();
        assert_eq!(theme.document.canvas.profile_name(), None);
        assert_eq!(
            (theme.document.canvas.width, theme.document.canvas.height),
            (1234, 567)
        );
    }

    #[test]
    fn every_official_profile_is_recognized_from_canvas_dimensions() {
        for ((width, height), expected_name) in [
            ((420, 64), "Slim"),
            ((420, 120), "Compact"),
            ((800, 150), "Standard"),
            ((960, 150), "Wide"),
            ((1000, 180), "Ultra-wide"),
        ] {
            let canvas = CanvasConfig {
                width,
                height,
                ..CanvasConfig::default()
            };
            assert_eq!(canvas.profile_name(), Some(expected_name));
        }
    }

    #[test]
    fn invalid_toml_required_fields_versions_and_compatibility_are_rejected() {
        assert!(matches!(
            loader().parse(b"[theme\nid = \"broken\""),
            Err(ThemeError::Toml(_))
        ));
        assert!(matches!(
            loader().parse(b"[theme]\nid = \"missing-fields\"\n"),
            Err(ThemeError::Toml(_))
        ));
        let unsupported = VALID_THEME.replace("format_version = 1", "format_version = 2");
        assert!(matches!(
            loader().parse(unsupported.as_bytes()),
            Err(ThemeError::UnsupportedFormat(2))
        ));
        let too_new =
            VALID_THEME.replace("minimum_runtime = \"1.0.0\"", "minimum_runtime = \"2.0.0\"");
        assert!(matches!(
            loader().parse(too_new.as_bytes()),
            Err(ThemeError::RuntimeTooOld { .. })
        ));
        let empty_html = VALID_THEME.replace("<main id='player'></main>", "  ");
        assert!(matches!(
            loader().parse(empty_html.as_bytes()),
            Err(ThemeError::InvalidManifest(_))
        ));
    }

    #[test]
    fn embedded_resource_validation_rejects_bad_base64_and_mime_types() {
        let bad_base64 =
            format!("{VALID_THEME}\n[assets.logo]\nmime_type = \"image/png\"\ndata = \"%%%\"\n");
        assert!(matches!(
            loader().parse(bad_base64.as_bytes()),
            Err(ThemeError::InvalidManifest(_))
        ));
        let bad_mime = format!(
            "{VALID_THEME}\n[assets.logo]\nmime_type = \"image/png; script=1\"\ndata = \"aGVsbG8=\"\n"
        );
        assert!(matches!(
            loader().parse(bad_mime.as_bytes()),
            Err(ThemeError::InvalidManifest(_))
        ));
        assert!(matches!(
            loader().parse(&vec![b'a'; MAX_THEME_FILE_BYTES + 1]),
            Err(ThemeError::TooLarge)
        ));
    }

    #[tokio::test]
    async fn discovery_reports_bad_files_and_keeps_other_themes_available() {
        let dir = TestDirectory::new("discovery");
        tokio::fs::write(dir.0.join("valid.toml"), VALID_THEME)
            .await
            .unwrap();
        tokio::fs::write(dir.0.join("invalid.toml"), "not = [valid")
            .await
            .unwrap();
        tokio::fs::write(dir.0.join("notes.txt"), "ignored")
            .await
            .unwrap();
        let found = loader().discover(&dir.0).await.unwrap();
        assert!(found.themes.contains_key("sample-player"));
        assert_eq!(found.issues.len(), 1);
        assert_eq!(found.issues[0].path.file_name().unwrap(), "invalid.toml");
    }

    #[tokio::test]
    async fn successful_theme_selection_notifies_connected_clients() {
        let dir = TestDirectory::new("theme-change");
        let source = dir.0.join("source.toml");
        tokio::fs::write(&source, VALID_THEME).await.unwrap();
        let manager =
            ThemeManager::new(dir.0.join("themes"), dir.0.join("settings.toml"), loader())
                .await
                .unwrap();
        manager.import_file(&source).await.unwrap();
        let mut changes = manager.subscribe_changes();
        manager.select("sample-player").await.unwrap();
        changes.changed().await.unwrap();
        assert_eq!(*changes.borrow(), 1);
    }

    #[tokio::test]
    async fn import_export_and_selection_persistence_preserve_existing_data() {
        let dir = TestDirectory::new("manager");
        let source = dir.0.join("downloaded.toml");
        let themes_dir = dir.0.join("themes");
        let settings = dir.0.join("settings.toml");
        let export = dir.0.join("exported.toml");
        tokio::fs::write(&source, VALID_THEME).await.unwrap();
        tokio::fs::write(&settings, "http_port = 8765\n")
            .await
            .unwrap();

        let manager = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        assert!(manager.refresh().await.unwrap().themes.is_empty());
        let theme = manager.import_file(&source).await.unwrap();
        assert_eq!(theme.id(), "sample-player");
        manager.select("sample-player").await.unwrap();
        assert_eq!(
            read_selected_id(&settings).await.unwrap().as_deref(),
            Some("sample-player")
        );
        let saved_settings = tokio::fs::read_to_string(&settings).await.unwrap();
        assert!(saved_settings.contains("http_port = 8765"));

        manager.export_file("sample-player", &export).await.unwrap();
        assert_eq!(
            tokio::fs::read(&export).await.unwrap(),
            VALID_THEME.as_bytes()
        );
        assert!(matches!(
            manager.export_file("sample-player", &export).await,
            Err(ThemeError::AlreadyExists(_))
        ));
        assert!(matches!(
            manager.import_file(&source).await,
            Err(ThemeError::AlreadyExists(_))
        ));

        let restored = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        restored.refresh().await.unwrap();
        assert_eq!(
            restored.restore_selection().await.unwrap().unwrap().id(),
            "sample-player"
        );
        assert!(matches!(
            restored.select("missing-theme").await,
            Err(ThemeError::NotFound(_))
        ));
        assert_eq!(restored.active().await.unwrap().id(), "sample-player");

        let invalid = dir.0.join("invalid-import.toml");
        tokio::fs::write(&invalid, "[theme]\nid = \"sample-player\"")
            .await
            .unwrap();
        assert!(restored.import_file(&invalid).await.is_err());
        assert_eq!(
            tokio::fs::read(themes_dir.join("sample-player.toml"))
                .await
                .unwrap(),
            VALID_THEME.as_bytes()
        );
    }

    #[tokio::test]
    async fn importing_theme_text_uses_validation_and_preserves_existing_themes() {
        let dir = TestDirectory::new("import-content");
        let themes_dir = dir.0.join("themes");
        let manager = ThemeManager::new(&themes_dir, dir.0.join("settings.toml"), loader())
            .await
            .unwrap();

        assert_eq!(
            manager.import_content(VALID_THEME).await.unwrap().id(),
            "sample-player"
        );
        assert!(matches!(
            manager.import_content("[theme\nid = \"broken\"").await,
            Err(ThemeError::Toml(_))
        ));
        let invalid_schema = VALID_THEME.replace("format_version = 1", "format_version = 2");
        assert!(matches!(
            manager.import_content(&invalid_schema).await,
            Err(ThemeError::UnsupportedFormat(2))
        ));
        assert!(matches!(
            manager.import_content(VALID_THEME).await,
            Err(ThemeError::AlreadyExists(_))
        ));

        let stored = tokio::fs::read_to_string(themes_dir.join("sample-player.toml"))
            .await
            .unwrap();
        assert_eq!(stored, VALID_THEME);
        assert_eq!(manager.catalog().await.themes.len(), 1);
    }

    #[tokio::test]
    async fn malformed_settings_are_preserved_and_failed_switch_keeps_active_theme() {
        let dir = TestDirectory::new("settings");
        let source = dir.0.join("sample.toml");
        let themes_dir = dir.0.join("themes");
        let settings = dir.0.join("settings.toml");
        tokio::fs::write(&source, VALID_THEME).await.unwrap();
        let manager = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        manager.import_file(&source).await.unwrap();
        manager.select("sample-player").await.unwrap();

        tokio::fs::write(&settings, "not = [valid").await.unwrap();
        assert!(matches!(
            manager.select("sample-player").await,
            Err(ThemeError::InvalidSettings)
        ));
        assert_eq!(manager.active().await.unwrap().id(), "sample-player");
        assert_eq!(
            tokio::fs::read_to_string(&settings).await.unwrap(),
            "not = [valid"
        );
    }

    #[tokio::test]
    async fn startup_can_activate_a_valid_theme_without_overwriting_bad_settings() {
        let dir = TestDirectory::new("ephemeral-activation");
        let source = dir.0.join("sample.toml");
        let themes_dir = dir.0.join("themes");
        let settings = dir.0.join("settings.toml");
        tokio::fs::write(&source, VALID_THEME).await.unwrap();
        tokio::fs::write(&settings, "not = [valid").await.unwrap();
        let manager = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        manager.import_file(&source).await.unwrap();

        manager.activate("sample-player").await.unwrap();

        assert_eq!(manager.active().await.unwrap().id(), "sample-player");
        assert_eq!(
            tokio::fs::read_to_string(&settings).await.unwrap(),
            "not = [valid"
        );
    }
}
