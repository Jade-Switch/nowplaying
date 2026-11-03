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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_id: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSelectorGroup {
    pub key: String,
    pub family_id: Option<String>,
    pub label: String,
    pub theme_ids: Vec<String>,
    pub warning: Option<String>,
}

impl ThemeSelectorGroup {
    pub fn preferred_variant_id(
        &self,
        active_theme_id: Option<&str>,
        remembered_variants: &BTreeMap<String, String>,
    ) -> Option<String> {
        if let Some(active_theme_id) = active_theme_id
            && self.theme_ids.iter().any(|id| id == active_theme_id)
        {
            return Some(active_theme_id.to_owned());
        }
        if let Some(family_id) = &self.family_id
            && let Some(remembered) = remembered_variants.get(family_id)
            && self.theme_ids.contains(remembered)
        {
            return Some(remembered.clone());
        }
        self.theme_ids.first().cloned()
    }
}

impl ThemeDiscovery {
    /// Groups compatible family members and keeps malformed families visible
    /// as separate entries with a warning.
    pub fn selector_groups(&self) -> Vec<ThemeSelectorGroup> {
        let mut families: BTreeMap<String, Vec<&Arc<LoadedTheme>>> = BTreeMap::new();
        let mut groups = Vec::new();

        for (id, theme) in &self.themes {
            if let Some(family_id) = &theme.document.theme.family_id {
                families.entry(family_id.clone()).or_default().push(theme);
            } else {
                groups.push(ThemeSelectorGroup {
                    key: id.clone(),
                    family_id: None,
                    label: theme.document.theme.name.clone(),
                    theme_ids: vec![id.clone()],
                    warning: None,
                });
            }
        }

        for (family_id, mut variants) in families {
            variants.sort_by_key(|theme| variant_sort_key(theme));
            let name = variants[0].document.theme.name.clone();
            let family_name_matches = variants
                .iter()
                .all(|theme| theme.document.theme.name == name);
            let authors: std::collections::BTreeSet<_> = variants
                .iter()
                .filter_map(|theme| theme.document.theme.author.as_deref())
                .collect();
            let authors_match = authors.len() <= 1;

            if !family_name_matches || !authors_match {
                let warning = if !family_name_matches && !authors_match {
                    format!(
                        "Family {family_id} has mismatched names and authors; variants are listed separately."
                    )
                } else if !family_name_matches {
                    format!(
                        "Family {family_id} has mismatched display names; variants are listed separately."
                    )
                } else {
                    format!(
                        "Family {family_id} has incompatible authors; variants are listed separately."
                    )
                };
                for theme in variants {
                    let id = theme.id().to_owned();
                    groups.push(ThemeSelectorGroup {
                        key: id.clone(),
                        family_id: None,
                        label: format!("{} ({id})", theme.document.theme.name),
                        theme_ids: vec![id],
                        warning: Some(warning.clone()),
                    });
                }
                continue;
            }

            let mut dimensions = BTreeMap::new();
            for theme in &variants {
                let canvas = &theme.document.canvas;
                *dimensions.entry((canvas.width, canvas.height)).or_insert(0) += 1;
            }
            let duplicate_dimensions = dimensions.values().any(|count| *count > 1);
            let theme_ids = variants.iter().map(|theme| theme.id().to_owned()).collect();
            groups.push(ThemeSelectorGroup {
                key: family_id.clone(),
                family_id: Some(family_id),
                label: name,
                theme_ids,
                warning: duplicate_dimensions.then(|| {
                    "Multiple variants use the same dimensions; their IDs distinguish them in the size menu.".to_owned()
                }),
            });
        }

        groups.sort_by(|left, right| {
            left.label
                .to_lowercase()
                .cmp(&right.label.to_lowercase())
                .then_with(|| left.key.cmp(&right.key))
        });
        groups
    }
}

fn variant_sort_key(theme: &LoadedTheme) -> (u8, u32, u32, String) {
    let canvas = &theme.document.canvas;
    let profile_order = match canvas.profile_name() {
        Some("Slim") => 0,
        Some("Compact") => 1,
        Some("Standard") => 2,
        Some("Wide") => 3,
        Some("Ultra-wide") => 4,
        _ => 5,
    };
    (
        profile_order,
        canvas.width,
        canvas.height,
        theme.id().to_owned(),
    )
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
    if identity
        .family_id
        .as_deref()
        .is_some_and(|family_id| !valid_theme_id(family_id))
    {
        return invalid("theme.family_id must use lowercase letters, digits and hyphens");
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
        let _mutation = self.mutation.lock().await;
        let discovery = self.loader.discover(&self.themes_dir).await?;
        *self.catalog.write().await = discovery.clone();
        let active = self.active.read().await.clone();
        if let Some(active) = active
            && !discovery.themes.contains_key(active.id())
            && let Some(family_id) = active.document.theme.family_id.as_deref()
            && let Some(group) = discovery
                .selector_groups()
                .into_iter()
                .find(|group| group.family_id.as_deref() == Some(family_id))
        {
            let remembered = self.selected_family_variants().await;
            if let Some(fallback_id) = group.preferred_variant_id(None, &remembered)
                && let Some(fallback) = discovery.themes.get(&fallback_id).cloned()
            {
                *self.active.write().await = Some(fallback);
                self.notify_theme_change();
            }
        }
        Ok(discovery)
    }

    pub async fn catalog(&self) -> ThemeDiscovery {
        self.catalog.read().await.clone()
    }

    pub async fn selected_family_variants(&self) -> BTreeMap<String, String> {
        read_selected_family_variants(&self.settings_path)
            .await
            .unwrap_or_default()
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
        save_selected_id(
            &self.settings_path,
            id,
            theme.document.theme.family_id.as_deref(),
        )
        .await?;
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
        if self.catalog.read().await.themes.contains_key(&id) {
            return self.select(&id).await.map(Some);
        }

        let family_id = read_selected_family_id(&self.settings_path).await?;
        if let Some(family_id) = family_id {
            let catalog = self.catalog.read().await.clone();
            if let Some(group) = catalog
                .selector_groups()
                .into_iter()
                .find(|group| group.family_id.as_deref() == Some(&family_id))
            {
                let remembered = self
                    .selected_family_variants()
                    .await
                    .get(&family_id)
                    .filter(|variant_id| group.theme_ids.contains(variant_id))
                    .cloned();
                let fallback_id = remembered.or_else(|| group.theme_ids.first().cloned());
                if let Some(fallback_id) = fallback_id {
                    return self.activate(&fallback_id).await.map(Some);
                }
            }
        }

        Err(ThemeError::NotFound(id))
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

async fn read_selected_family_id(path: &Path) -> Result<Option<String>, ThemeError> {
    let bytes = match read_limited(path, MAX_SETTINGS_BYTES).await {
        Ok(bytes) => bytes,
        Err(ThemeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(ThemeError::TooLarge) => return Err(ThemeError::InvalidSettings),
        Err(error) => return Err(error),
    };
    let value: toml::Value = toml::from_slice(&bytes).map_err(|_| ThemeError::InvalidSettings)?;
    Ok(value
        .as_table()
        .and_then(|table| table.get("selected_theme_family_id"))
        .and_then(toml::Value::as_str)
        .filter(|family_id| valid_theme_id(family_id))
        .map(str::to_owned))
}

async fn read_selected_family_variants(
    path: &Path,
) -> Result<BTreeMap<String, String>, ThemeError> {
    let bytes = match read_limited(path, MAX_SETTINGS_BYTES).await {
        Ok(bytes) => bytes,
        Err(ThemeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(ThemeError::TooLarge) => return Err(ThemeError::InvalidSettings),
        Err(error) => return Err(error),
    };
    let value: toml::Value = toml::from_slice(&bytes).map_err(|_| ThemeError::InvalidSettings)?;
    let mut variants = BTreeMap::new();
    if let Some(entries) = value
        .as_table()
        .and_then(|table| table.get("selected_theme_variants"))
        .and_then(toml::Value::as_table)
    {
        for (family_id, theme_id) in entries {
            if valid_theme_id(family_id)
                && let Some(theme_id) = theme_id.as_str()
                && valid_theme_id(theme_id)
            {
                variants.insert(family_id.clone(), theme_id.to_owned());
            }
        }
    }
    Ok(variants)
}

async fn save_selected_id(
    path: &Path,
    id: &str,
    family_id: Option<&str>,
) -> Result<(), ThemeError> {
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
    if let Some(family_id) = family_id {
        if !valid_theme_id(family_id) {
            return Err(ThemeError::InvalidSettings);
        }
        table.insert(
            "selected_theme_family_id".into(),
            toml::Value::String(family_id.to_owned()),
        );
        let variants = table
            .entry("selected_theme_variants")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or(ThemeError::InvalidSettings)?;
        variants.insert(family_id.to_owned(), toml::Value::String(id.to_owned()));
    } else {
        table.remove("selected_theme_family_id");
    }
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

    fn family_theme(
        id: &str,
        name: &str,
        family_id: Option<&str>,
        author: Option<&str>,
        width: u32,
        height: u32,
    ) -> String {
        let family = family_id
            .map(|family_id| format!("family_id = \"{family_id}\"\n"))
            .unwrap_or_default();
        let author = author
            .map(|author| format!("author = \"{author}\"\n"))
            .unwrap_or_default();
        format!(
            "[theme]\nid = \"{id}\"\nname = \"{name}\"\n{family}version = \"1.0.0\"\nformat_version = 1\n{author}\n[theme.compatibility]\nminimum_runtime = \"1.0.0\"\n\n[canvas]\nwidth = {width}\nheight = {height}\n\n[html]\ntemplate = \"<main>{id}</main>\"\n\n[css]\nstylesheet = \".root {{ width: {width}px; height: {height}px; }}\"\n"
        )
    }

    fn discovery_from_themes(sources: &[String]) -> ThemeDiscovery {
        let mut discovery = ThemeDiscovery::default();
        for source in sources {
            let theme = Arc::new(loader().parse(source.as_bytes()).unwrap());
            discovery.themes.insert(theme.id().to_owned(), theme);
        }
        discovery
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
    fn family_metadata_is_optional_validated_and_keeps_legacy_themes_independent() {
        let legacy = loader().parse(VALID_THEME.as_bytes()).unwrap();
        assert_eq!(legacy.document.theme.family_id, None);
        let mut invalid = family_theme("bad-family", "Bad Family", Some("Bad-ID"), None, 420, 120);
        assert!(matches!(
            loader().parse(invalid.as_bytes()),
            Err(ThemeError::InvalidManifest(_))
        ));
        invalid = family_theme(
            "valid-family",
            "Valid Family",
            Some("valid-family"),
            None,
            420,
            120,
        );
        assert_eq!(
            loader()
                .parse(invalid.as_bytes())
                .unwrap()
                .document
                .theme
                .family_id
                .as_deref(),
            Some("valid-family")
        );
    }

    #[test]
    fn family_groups_compact_and_wide_variants_once_in_profile_order() {
        let discovery = discovery_from_themes(&[
            family_theme(
                "dreamwave-wide",
                "Dreamwave",
                Some("dreamwave"),
                Some("NPS"),
                960,
                150,
            ),
            family_theme(
                "dreamwave-compact",
                "Dreamwave",
                Some("dreamwave"),
                Some("NPS"),
                420,
                120,
            ),
        ]);
        let groups = discovery.selector_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Dreamwave");
        assert_eq!(groups[0].family_id.as_deref(), Some("dreamwave"));
        assert_eq!(groups[0].theme_ids, ["dreamwave-compact", "dreamwave-wide"]);
        assert_eq!(groups[0].warning, None);
    }

    #[test]
    fn standalone_themes_with_the_same_display_name_remain_separate() {
        let discovery = discovery_from_themes(&[
            family_theme("same-name-a", "Same Name", None, None, 420, 120),
            family_theme("same-name-b", "Same Name", None, None, 960, 150),
        ]);
        let groups = discovery.selector_groups();
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|group| group.family_id.is_none()));
        assert!(groups.iter().all(|group| group.theme_ids.len() == 1));
    }

    #[test]
    fn family_can_contain_all_official_profiles() {
        let sources = [
            ("slim", 420, 64),
            ("compact", 420, 120),
            ("standard", 800, 150),
            ("wide", 960, 150),
            ("ultra", 1000, 180),
        ]
        .into_iter()
        .map(|(suffix, width, height)| {
            family_theme(
                &format!("orbit-{suffix}"),
                "Orbit",
                Some("orbit"),
                None,
                width,
                height,
            )
        })
        .collect::<Vec<_>>();
        let groups = discovery_from_themes(&sources).selector_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].theme_ids.len(), 5);
    }

    #[test]
    fn singleton_families_custom_sizes_and_multiple_families_are_preserved() {
        let discovery = discovery_from_themes(&[
            family_theme("single-one", "Single", Some("single"), None, 800, 150),
            family_theme("custom-a", "Custom", Some("custom"), None, 700, 200),
            family_theme("custom-b", "Custom", Some("custom"), None, 730, 210),
            family_theme("other-a", "Other", Some("other"), None, 420, 120),
            VALID_THEME.to_owned(),
        ]);
        let groups = discovery.selector_groups();
        assert_eq!(groups.len(), 4);
        let single = groups.iter().find(|group| group.key == "single").unwrap();
        assert_eq!(single.theme_ids.len(), 1);
        let custom = groups.iter().find(|group| group.key == "custom").unwrap();
        assert_eq!(custom.theme_ids.len(), 2);
        assert!(groups.iter().any(|group| group.key == "sample-player"));
        assert!(groups.iter().any(|group| group.key == "other"));
    }

    #[test]
    fn family_default_uses_remembered_variant_and_recovers_from_stale_ids() {
        let discovery = discovery_from_themes(&[
            family_theme("return-compact", "Return", Some("return"), None, 420, 120),
            family_theme("return-wide", "Return", Some("return"), None, 960, 150),
        ]);
        let groups = discovery.selector_groups();
        let group = groups.iter().find(|group| group.key == "return").unwrap();
        let remembered = BTreeMap::from([("return".to_owned(), "return-wide".to_owned())]);
        assert_eq!(
            group.preferred_variant_id(Some("another-theme"), &remembered),
            Some("return-wide".to_owned())
        );
        assert_eq!(
            group.preferred_variant_id(Some("return-compact"), &remembered),
            Some("return-compact".to_owned())
        );
        let stale = BTreeMap::from([("return".to_owned(), "removed-variant".to_owned())]);
        assert_eq!(
            group.preferred_variant_id(None, &stale),
            Some("return-compact".to_owned())
        );
    }

    #[test]
    fn inconsistent_family_metadata_stays_visible_as_separate_entries() {
        let discovery = discovery_from_themes(&[
            family_theme(
                "name-one",
                "First",
                Some("conflict"),
                Some("Author"),
                420,
                120,
            ),
            family_theme(
                "name-two",
                "Second",
                Some("conflict"),
                Some("Author"),
                960,
                150,
            ),
            family_theme(
                "author-one",
                "Authors",
                Some("authors"),
                Some("A"),
                800,
                150,
            ),
            family_theme(
                "author-two",
                "Authors",
                Some("authors"),
                Some("B"),
                1000,
                180,
            ),
        ]);
        let groups = discovery.selector_groups();
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|group| group.theme_ids.len() == 1));
        assert!(groups.iter().all(|group| group.warning.is_some()));
    }

    #[test]
    fn duplicate_family_dimensions_are_reported_without_hiding_variants() {
        let discovery = discovery_from_themes(&[
            family_theme(
                "duplicate-a",
                "Duplicate",
                Some("duplicate"),
                None,
                420,
                120,
            ),
            family_theme(
                "duplicate-b",
                "Duplicate",
                Some("duplicate"),
                None,
                420,
                120,
            ),
        ]);
        let groups = discovery.selector_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].theme_ids, ["duplicate-a", "duplicate-b"]);
        assert!(
            groups[0]
                .warning
                .as_deref()
                .unwrap()
                .contains("same dimensions")
        );
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
        tokio::fs::write(
            dir.0.join("invalid-family.toml"),
            family_theme(
                "invalid-family",
                "Invalid Family",
                Some("Bad-ID"),
                None,
                420,
                120,
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(dir.0.join("notes.txt"), "ignored")
            .await
            .unwrap();
        let found = loader().discover(&dir.0).await.unwrap();
        assert!(found.themes.contains_key("sample-player"));
        assert_eq!(found.issues.len(), 2);
        assert!(
            found
                .issues
                .iter()
                .any(|issue| { issue.path.file_name().unwrap() == "invalid.toml" })
        );
        assert!(
            found
                .issues
                .iter()
                .any(|issue| { issue.path.file_name().unwrap() == "invalid-family.toml" })
        );
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
    async fn family_variant_selection_persists_by_theme_id_and_restores() {
        let dir = TestDirectory::new("family-persist");
        let themes_dir = dir.0.join("themes");
        let settings = dir.0.join("settings.toml");
        tokio::fs::create_dir_all(&themes_dir).await.unwrap();
        tokio::fs::write(
            themes_dir.join("dreamwave-compact.toml"),
            family_theme(
                "dreamwave-compact",
                "Dreamwave",
                Some("dreamwave"),
                None,
                420,
                120,
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(
            themes_dir.join("dreamwave-wide.toml"),
            family_theme(
                "dreamwave-wide",
                "Dreamwave",
                Some("dreamwave"),
                None,
                960,
                150,
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(themes_dir.join("sample-player.toml"), VALID_THEME)
            .await
            .unwrap();

        let manager = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        manager.refresh().await.unwrap();
        manager.select("dreamwave-wide").await.unwrap();
        assert_eq!(
            read_selected_id(&settings).await.unwrap().as_deref(),
            Some("dreamwave-wide")
        );
        assert_eq!(
            manager
                .selected_family_variants()
                .await
                .get("dreamwave")
                .map(String::as_str),
            Some("dreamwave-wide")
        );

        let restored = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        restored.refresh().await.unwrap();
        assert_eq!(
            restored.restore_selection().await.unwrap().unwrap().id(),
            "dreamwave-wide"
        );
        restored.select("sample-player").await.unwrap();
        assert_eq!(
            restored
                .selected_family_variants()
                .await
                .get("dreamwave")
                .map(String::as_str),
            Some("dreamwave-wide")
        );
    }

    #[tokio::test]
    async fn missing_selected_family_variant_falls_back_without_rewriting_choice() {
        let dir = TestDirectory::new("family-fallback");
        let themes_dir = dir.0.join("themes");
        let settings = dir.0.join("settings.toml");
        tokio::fs::create_dir_all(&themes_dir).await.unwrap();
        tokio::fs::write(
            themes_dir.join("dreamwave-compact.toml"),
            family_theme(
                "dreamwave-compact",
                "Dreamwave",
                Some("dreamwave"),
                None,
                420,
                120,
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(
            themes_dir.join("dreamwave-wide.toml"),
            family_theme(
                "dreamwave-wide",
                "Dreamwave",
                Some("dreamwave"),
                None,
                960,
                150,
            ),
        )
        .await
        .unwrap();
        let manager = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        manager.refresh().await.unwrap();
        manager.select("dreamwave-wide").await.unwrap();
        tokio::fs::remove_file(themes_dir.join("dreamwave-wide.toml"))
            .await
            .unwrap();
        manager.refresh().await.unwrap();
        assert_eq!(manager.active().await.unwrap().id(), "dreamwave-compact");
        assert_eq!(
            read_selected_id(&settings).await.unwrap().as_deref(),
            Some("dreamwave-wide")
        );

        let restored = ThemeManager::new(&themes_dir, &settings, loader())
            .await
            .unwrap();
        restored.refresh().await.unwrap();
        assert_eq!(
            restored.restore_selection().await.unwrap().unwrap().id(),
            "dreamwave-compact"
        );
        assert_eq!(
            read_selected_id(&settings).await.unwrap().as_deref(),
            Some("dreamwave-wide")
        );
    }

    #[tokio::test]
    async fn imported_theme_can_join_existing_family_and_preview_uses_variant_content() {
        let dir = TestDirectory::new("family-import");
        let themes_dir = dir.0.join("themes");
        let manager = ThemeManager::new(&themes_dir, dir.0.join("settings.toml"), loader())
            .await
            .unwrap();
        let compact = family_theme(
            "dreamwave-compact",
            "Dreamwave",
            Some("dreamwave"),
            None,
            420,
            120,
        );
        manager.import_content(&compact).await.unwrap();
        let wide = family_theme(
            "dreamwave-wide",
            "Dreamwave",
            Some("dreamwave"),
            None,
            960,
            150,
        );
        manager.import_content(&wide).await.unwrap();

        let group = manager
            .catalog()
            .await
            .selector_groups()
            .into_iter()
            .find(|group| group.family_id.as_deref() == Some("dreamwave"))
            .unwrap();
        assert_eq!(group.theme_ids.len(), 2);

        let compact = manager.select("dreamwave-compact").await.unwrap();
        let compact_preview =
            crate::runtime::render_theme(&compact, "dGhlbWVub25jZXRoZW1lbm9uY2U").unwrap();
        assert!(
            compact_preview
                .html
                .contains("<main>dreamwave-compact</main>")
        );
        assert!(compact_preview.html.contains("width: 420px;"));

        let selected = manager.select("dreamwave-wide").await.unwrap();
        let preview =
            crate::runtime::render_theme(&selected, "dGhlbWVub25jZXRoZW1lbm9uY2U").unwrap();
        assert!(preview.html.contains("<main>dreamwave-wide</main>"));
        assert!(preview.html.contains("width: 960px;"));
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
