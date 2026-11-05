use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, MutexGuard},
};
use tracing::warn;
use url::{Host, Url};

use crate::playback::TrackMetadata;

const SOURCE_LOCK_LIMIT: usize = 256;
const SOURCE_INDEX_FACTOR: usize = 4;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

const FALLBACK_ID: &str = "fallback";
const FALLBACK_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#cba6f7"/><stop offset="1" stop-color="#313244"/></linearGradient></defs><rect width="256" height="256" rx="28" fill="url(#g)"/><circle cx="128" cy="128" r="57" fill="#181825"/><circle cx="128" cy="128" r="11" fill="#b4befe"/><path d="M128 71v57l39 22" fill="none" stroke="#cdd6f4" stroke-linecap="round" stroke-width="9"/></svg>"##;

#[derive(Debug, Clone)]
pub struct ArtworkCacheConfig {
    pub max_artwork_bytes: usize,
    pub max_cache_bytes: usize,
    pub max_entries: usize,
    /// Local MPRIS art is read only from canonical paths below these roots.
    pub allowed_local_roots: Vec<PathBuf>,
}

impl Default for ArtworkCacheConfig {
    fn default() -> Self {
        let allowed_local_roots = std::env::var_os("HOME")
            .map(PathBuf::from)
            .into_iter()
            .collect();
        Self {
            max_artwork_bytes: 8 * 1024 * 1024,
            max_cache_bytes: 64 * 1024 * 1024,
            max_entries: 256,
            allowed_local_roots,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkRef {
    /// SHA-256 content ID, or the reserved `fallback` ID.
    pub id: String,
    /// Stable path for the future local HTTP service.
    pub local_url: String,
    pub mime_type: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkFile {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkCacheStats {
    pub entries: usize,
    pub bytes: usize,
}

#[derive(Debug, Error)]
pub enum ArtworkError {
    #[error("invalid artwork URI")]
    InvalidUri,
    #[error("unsupported artwork URI scheme")]
    UnsupportedScheme,
    #[error("local artwork is outside the allowed directories")]
    LocalPathDenied,
    #[error("artwork source resolves to a local or reserved network address")]
    AddressDenied,
    #[error("artwork exceeds the configured size limit")]
    TooLarge,
    #[error("artwork data is not a supported raster image")]
    InvalidImage,
    #[error("invalid artwork content ID")]
    InvalidId,
    #[error("artwork cache configuration is invalid")]
    InvalidConfig,
    #[error("artwork download failed")]
    Download(#[source] reqwest::Error),
    #[error("artwork server returned an unsuccessful HTTP status")]
    DownloadStatus,
    #[error("artwork storage failed")]
    Io(#[source] std::io::Error),
}

#[derive(Debug, Clone)]
struct CacheEntry {
    path: PathBuf,
    mime_type: &'static str,
    size: usize,
    last_used: SystemTime,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CacheEntry>,
    source_ids: HashMap<String, String>,
    source_order: VecDeque<String>,
    bytes: usize,
}

struct SourceFlight {
    gate: std::sync::Arc<Mutex<()>>,
}

enum ArtworkSource {
    Local { key: String, path: PathBuf },
    Remote { key: String, url: Url },
}

impl ArtworkSource {
    fn key(&self) -> &str {
        match self {
            Self::Local { key, .. } | Self::Remote { key, .. } => key,
        }
    }
}

pub struct ArtworkCache {
    cache_dir: PathBuf,
    allowed_local_roots: Vec<PathBuf>,
    config: ArtworkCacheConfig,
    state: Mutex<CacheState>,
    source_flights: Mutex<HashMap<String, std::sync::Arc<SourceFlight>>>,
}

impl ArtworkCache {
    pub async fn new(
        cache_dir: impl AsRef<Path>,
        config: ArtworkCacheConfig,
    ) -> Result<Self, ArtworkError> {
        if config.max_artwork_bytes == 0
            || config.max_cache_bytes == 0
            || config.max_entries == 0
            || config.max_entries > 4096
            || config.max_artwork_bytes > 128 * 1024 * 1024
            || config.max_cache_bytes > 1024 * 1024 * 1024
        {
            return Err(ArtworkError::InvalidConfig);
        }
        tokio::fs::create_dir_all(cache_dir.as_ref())
            .await
            .map_err(ArtworkError::Io)?;
        let cache_dir = tokio::fs::canonicalize(cache_dir.as_ref())
            .await
            .map_err(ArtworkError::Io)?;
        let mut allowed_local_roots = Vec::new();
        for root in config.allowed_local_roots.iter() {
            if let Ok(root) = tokio::fs::canonicalize(root).await
                && !allowed_local_roots.contains(&root)
            {
                allowed_local_roots.push(root);
            }
        }
        let cache = Self {
            cache_dir,
            allowed_local_roots,
            config,
            state: Mutex::new(CacheState::default()),
            source_flights: Mutex::new(HashMap::new()),
        };
        cache.load_existing().await?;
        Ok(cache)
    }

    /// Resolve and cache the MPRIS artwork URI for a track.
    pub async fn resolve_track(
        &self,
        metadata: &TrackMetadata,
    ) -> Result<Option<ArtworkRef>, ArtworkError> {
        match metadata.artwork_uri.as_deref() {
            Some(uri) => self.resolve(uri).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn resolve(&self, uri: &str) -> Result<ArtworkRef, ArtworkError> {
        let source = self.classify_source(uri).await?;
        let _flight = self.acquire_source_flight(source.key()).await;
        if let Some(artwork) = self.cached_source(source.key()).await {
            return Ok(artwork);
        }
        let bytes = match &source {
            ArtworkSource::Local { path, .. } => self.read_local(path).await?,
            ArtworkSource::Remote { url, .. } => self.download(url).await?,
        };
        self.store(source.key(), bytes).await
    }

    /// Return the bundled fallback when artwork is missing or cannot be read.
    pub async fn resolve_or_fallback(&self, uri: Option<&str>) -> ArtworkRef {
        match uri {
            Some(uri) => match self.resolve(uri).await {
                Ok(artwork) => artwork,
                Err(error) => {
                    warn!(%error, "Could not load track artwork; using fallback");
                    Self::fallback()
                }
            },
            None => Self::fallback(),
        }
    }

    pub fn fallback() -> ArtworkRef {
        ArtworkRef {
            id: FALLBACK_ID.to_owned(),
            local_url: format!("/artwork/{FALLBACK_ID}"),
            mime_type: "image/svg+xml".to_owned(),
            size_bytes: FALLBACK_SVG.len(),
        }
    }

    /// Read an opaque content ID for the future HTTP endpoint.
    pub async fn read(&self, id: &str) -> Result<Option<ArtworkFile>, ArtworkError> {
        if id == FALLBACK_ID {
            return Ok(Some(ArtworkFile {
                mime_type: "image/svg+xml".to_owned(),
                bytes: FALLBACK_SVG.to_vec(),
            }));
        }
        if !is_content_id(id) {
            return Err(ArtworkError::InvalidId);
        }
        let (file, mime_type) = {
            let mut state = self.state.lock().await;
            let Some(entry) = state.entries.get_mut(id) else {
                return Ok(None);
            };
            entry.last_used = SystemTime::now();
            let mime_type = entry.mime_type.to_owned();
            let file = tokio::fs::File::open(&entry.path)
                .await
                .map_err(ArtworkError::Io)?;
            (file, mime_type)
        };
        let bytes = read_limited_file_handle(file, self.config.max_artwork_bytes).await?;
        Ok(Some(ArtworkFile { mime_type, bytes }))
    }

    pub async fn stats(&self) -> ArtworkCacheStats {
        let state = self.state.lock().await;
        ArtworkCacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
        }
    }

    async fn classify_source(&self, uri: &str) -> Result<ArtworkSource, ArtworkError> {
        if Path::new(uri).is_absolute() {
            let path = self.canonical_local_path(Path::new(uri)).await?;
            return Ok(ArtworkSource::Local {
                key: format!("file:{}", path.display()),
                path,
            });
        }
        let parsed = Url::parse(uri).map_err(|_| ArtworkError::InvalidUri)?;
        match parsed.scheme() {
            "file" => {
                if parsed.query().is_some() || parsed.fragment().is_some() {
                    return Err(ArtworkError::InvalidUri);
                }
                if parsed
                    .host_str()
                    .is_some_and(|host| !host.is_empty() && host != "localhost")
                {
                    return Err(ArtworkError::LocalPathDenied);
                }
                let path = parsed
                    .to_file_path()
                    .map_err(|_| ArtworkError::InvalidUri)?;
                let path = self.canonical_local_path(&path).await?;
                Ok(ArtworkSource::Local {
                    key: format!("file:{}", path.display()),
                    path,
                })
            }
            "http" | "https" => {
                let parsed = validate_remote_uri(uri)?;
                Ok(ArtworkSource::Remote {
                    key: format!("http:{}", parsed.as_str()),
                    url: parsed,
                })
            }
            _ => Err(ArtworkError::UnsupportedScheme),
        }
    }

    async fn canonical_local_path(&self, path: &Path) -> Result<PathBuf, ArtworkError> {
        let canonical = tokio::fs::canonicalize(path)
            .await
            .map_err(ArtworkError::Io)?;
        if !self
            .allowed_local_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            return Err(ArtworkError::LocalPathDenied);
        }
        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(ArtworkError::Io)?;
        if !metadata.is_file() {
            return Err(ArtworkError::InvalidUri);
        }
        if metadata.len() > self.config.max_artwork_bytes as u64 {
            return Err(ArtworkError::TooLarge);
        }
        Ok(canonical)
    }

    async fn read_local(&self, path: &Path) -> Result<Vec<u8>, ArtworkError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(ArtworkError::Io)?;
        let mut bytes = Vec::with_capacity(self.config.max_artwork_bytes.min(64 * 1024));
        file.take(self.config.max_artwork_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(ArtworkError::Io)?;
        if bytes.len() > self.config.max_artwork_bytes {
            return Err(ArtworkError::TooLarge);
        }
        Ok(bytes)
    }

    async fn download(&self, url: &Url) -> Result<Vec<u8>, ArtworkError> {
        let host = url.host_str().ok_or(ArtworkError::InvalidUri)?;
        let port = url
            .port_or_known_default()
            .ok_or(ArtworkError::InvalidUri)?;
        let addresses = resolve_public_addresses(host, port).await?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!(
                "nowplaying/",
                env!("CARGO_PKG_VERSION"),
                " artwork"
            ))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(ArtworkError::Download)?;
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "image/*")
            .send()
            .await
            .map_err(ArtworkError::Download)?;
        if !response.status().is_success() {
            return Err(ArtworkError::DownloadStatus);
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.config.max_artwork_bytes as u64)
        {
            return Err(ArtworkError::TooLarge);
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ArtworkError::Download)?;
            if bytes.len().saturating_add(chunk.len()) > self.config.max_artwork_bytes {
                return Err(ArtworkError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn acquire_source_flight(
        &self,
        key: &str,
    ) -> Option<(
        std::sync::Arc<SourceFlight>,
        tokio::sync::OwnedMutexGuard<()>,
    )> {
        let flight = {
            let mut flights = self.source_flights.lock().await;
            if let Some(flight) = flights.get(key) {
                Some(flight.clone())
            } else {
                if flights.len() >= SOURCE_LOCK_LIMIT
                    && let Some(evict) = flights
                        .iter()
                        .find(|(_, flight)| std::sync::Arc::strong_count(flight) == 1)
                        .map(|(key, _)| key.clone())
                {
                    flights.remove(&evict);
                }
                if flights.len() >= SOURCE_LOCK_LIMIT {
                    None
                } else {
                    let flight = std::sync::Arc::new(SourceFlight {
                        gate: std::sync::Arc::new(Mutex::new(())),
                    });
                    flights.insert(key.to_owned(), flight.clone());
                    Some(flight)
                }
            }
        }?;
        let guard = flight.gate.clone().lock_owned().await;
        Some((flight, guard))
    }

    async fn cached_source(&self, source_key: &str) -> Option<ArtworkRef> {
        let mut state = self.state.lock().await;
        let id = state.source_ids.get(source_key)?.clone();
        let Some(entry) = state.entries.get_mut(&id) else {
            state.source_ids.remove(source_key);
            return None;
        };
        entry.last_used = SystemTime::now();
        let reference = artwork_ref(&id, entry);
        index_source(
            &mut state,
            source_key,
            &id,
            self.config.max_entries.saturating_mul(SOURCE_INDEX_FACTOR),
        );
        Some(reference)
    }

    async fn store(&self, source_key: &str, bytes: Vec<u8>) -> Result<ArtworkRef, ArtworkError> {
        if bytes.len() > self.config.max_artwork_bytes || bytes.len() > self.config.max_cache_bytes
        {
            return Err(ArtworkError::TooLarge);
        }
        let (mime_type, extension) = image_type(&bytes).ok_or(ArtworkError::InvalidImage)?;
        let id = content_id(&bytes);
        let now = SystemTime::now();
        let mut state = self.state.lock().await;
        if let Some(entry) = state.entries.get_mut(&id) {
            entry.last_used = now;
            let entry = entry.clone();
            index_source(
                &mut state,
                source_key,
                &id,
                self.config.max_entries.saturating_mul(SOURCE_INDEX_FACTOR),
            );
            return Ok(artwork_ref(&id, &entry));
        }

        while state.entries.len() >= self.config.max_entries
            || state.bytes.saturating_add(bytes.len()) > self.config.max_cache_bytes
        {
            evict_oldest(&mut state).await;
        }

        let final_path = self.cache_dir.join(format!("{id}.{extension}"));
        let temp_path = self.cache_dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await
            .map_err(ArtworkError::Io)?;
        if let Err(error) = file.write_all(&bytes).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(ArtworkError::Io(error));
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(ArtworkError::Io(error));
        }
        drop(file);
        if let Err(error) = tokio::fs::rename(&temp_path, &final_path).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(ArtworkError::Io(error));
        }
        let entry = CacheEntry {
            path: final_path,
            mime_type,
            size: bytes.len(),
            last_used: now,
        };
        state.bytes += entry.size;
        state.entries.insert(id.clone(), entry.clone());
        index_source(
            &mut state,
            source_key,
            &id,
            self.config.max_entries.saturating_mul(SOURCE_INDEX_FACTOR),
        );
        Ok(artwork_ref(&id, &entry))
    }

    async fn load_existing(&self) -> Result<(), ArtworkError> {
        let mut directory = tokio::fs::read_dir(&self.cache_dir)
            .await
            .map_err(ArtworkError::Io)?;
        let mut candidates = Vec::new();
        while let Some(entry) = directory.next_entry().await.map_err(ArtworkError::Io)? {
            let file_type = entry.file_type().await.map_err(ArtworkError::Io)?;
            if !file_type.is_file() {
                continue;
            }
            let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if filename.starts_with(".tmp-") {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            }
            let Some((id, extension)) = filename.rsplit_once('.') else {
                continue;
            };
            if !is_content_id(id) {
                continue;
            }
            let metadata = entry.metadata().await.map_err(ArtworkError::Io)?;
            if metadata.len() > self.config.max_artwork_bytes as u64 {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            }
            let bytes = match read_limited_file(&entry.path(), self.config.max_artwork_bytes).await
            {
                Ok(bytes) => bytes,
                Err(_) => {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                    continue;
                }
            };
            let Some((mime_type, actual_extension)) = image_type(&bytes) else {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            };
            if actual_extension != extension || content_id(&bytes) != id {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            }
            candidates.push((
                id.to_owned(),
                CacheEntry {
                    path: entry.path(),
                    mime_type,
                    size: bytes.len(),
                    last_used: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                },
            ));
        }
        candidates.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_used));
        let mut state = self.state.lock().await;
        for (id, entry) in candidates {
            if state.entries.len() >= self.config.max_entries
                || state.bytes.saturating_add(entry.size) > self.config.max_cache_bytes
            {
                let _ = tokio::fs::remove_file(entry.path).await;
                continue;
            }
            state.bytes += entry.size;
            state.entries.insert(id, entry);
        }
        Ok(())
    }
}

fn artwork_ref(id: &str, entry: &CacheEntry) -> ArtworkRef {
    ArtworkRef {
        id: id.to_owned(),
        local_url: format!("/artwork/{id}"),
        mime_type: entry.mime_type.to_owned(),
        size_bytes: entry.size,
    }
}

fn index_source(state: &mut CacheState, source: &str, id: &str, max_sources: usize) {
    state.source_ids.insert(source.to_owned(), id.to_owned());
    state.source_order.retain(|item| item != source);
    state.source_order.push_back(source.to_owned());
    while state.source_order.len() > max_sources {
        if let Some(oldest) = state.source_order.pop_front() {
            state.source_ids.remove(&oldest);
        }
    }
}

async fn evict_oldest(state: &mut MutexGuard<'_, CacheState>) {
    let oldest_id = state
        .entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_used)
        .map(|(id, _)| id.clone());
    let Some(id) = oldest_id else {
        return;
    };
    if let Some(entry) = state.entries.remove(&id) {
        state.bytes = state.bytes.saturating_sub(entry.size);
        let _ = tokio::fs::remove_file(entry.path).await;
        state.source_ids.retain(|_, cached_id| cached_id != &id);
    }
}

async fn read_limited_file(path: &Path, limit: usize) -> Result<Vec<u8>, ArtworkError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(ArtworkError::Io)?;
    read_limited_file_handle(file, limit).await
}

async fn read_limited_file_handle(
    file: tokio::fs::File,
    limit: usize,
) -> Result<Vec<u8>, ArtworkError> {
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(ArtworkError::Io)?;
    if bytes.len() > limit {
        return Err(ArtworkError::TooLarge);
    }
    Ok(bytes)
}

fn is_content_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn content_id(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut id = String::with_capacity(64);
    for byte in digest {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if bytes.starts_with(b"BM") {
        Some(("image/bmp", "bmp"))
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some(("image/x-icon", "ico"))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"avif" | b"avis")
    {
        Some(("image/avif", "avif"))
    } else {
        None
    }
}

async fn resolve_public_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, ArtworkError> {
    let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| ArtworkError::AddressDenied)?
            .collect(),
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_address(address.ip()))
    {
        return Err(ArtworkError::AddressDenied);
    }
    Ok(addresses)
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip == Ipv4Addr::BROADCAST
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_public_ipv4(ipv4);
    }
    let segments = ip.segments();
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001 && (segments[1] <= 0x01ff || segments[1] == 0x0db8))
        && segments[0] != 0x2002
}

/// Validate a remote URI without performing DNS or network access.
pub fn validate_remote_uri(uri: &str) -> Result<Url, ArtworkError> {
    let url = Url::parse(uri).map_err(|_| ArtworkError::InvalidUri)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ArtworkError::InvalidUri);
    }
    if url
        .port()
        .is_some_and(|port| port != if url.scheme() == "https" { 443 } else { 80 })
    {
        return Err(ArtworkError::InvalidUri);
    }
    if let Some(host) = url.host() {
        match host {
            Host::Domain(domain)
                if domain.eq_ignore_ascii_case("localhost")
                    || domain.ends_with(".localhost")
                    || domain.ends_with(".local") =>
            {
                return Err(ArtworkError::AddressDenied);
            }
            Host::Ipv4(ip) if !is_public_ipv4(ip) => return Err(ArtworkError::AddressDenied),
            Host::Ipv6(ip) if !is_public_ipv6(ip) => return Err(ArtworkError::AddressDenied),
            _ => {}
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_A: &[u8] = b"\x89PNG\r\n\x1a\nartwork-a";
    const PNG_B: &[u8] = b"\x89PNG\r\n\x1a\nartwork-b";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "nps-{label}-{}-{}",
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

    fn config(root: &Path) -> ArtworkCacheConfig {
        ArtworkCacheConfig {
            max_artwork_bytes: 1024,
            max_cache_bytes: 2048,
            max_entries: 4,
            allowed_local_roots: vec![root.to_owned()],
        }
    }

    #[tokio::test]
    async fn local_art_is_confined_cached_and_deduplicated_by_content() {
        let root = TestDirectory::new("allowed");
        let cache_dir = root.0.join("cache");
        let first = root.0.join("first.png");
        let second = root.0.join("second.png");
        tokio::fs::write(&first, PNG_A).await.unwrap();
        tokio::fs::write(&second, PNG_A).await.unwrap();
        let cache = ArtworkCache::new(&cache_dir, config(&root.0))
            .await
            .unwrap();

        let one = cache
            .resolve_track(&TrackMetadata {
                artwork_uri: Some(first.to_string_lossy().into_owned()),
                ..TrackMetadata::default()
            })
            .await
            .unwrap()
            .unwrap();
        let second_url = Url::from_file_path(&second).unwrap();
        let two = cache.resolve(second_url.as_str()).await.unwrap();
        assert_eq!(one.id, two.id);
        assert_eq!(cache.stats().await.entries, 1);
        assert_eq!(cache.read(&one.id).await.unwrap().unwrap().bytes, PNG_A);

        let reopened = ArtworkCache::new(&cache_dir, config(&root.0))
            .await
            .unwrap();
        assert_eq!(reopened.stats().await.entries, 1);
        assert_eq!(reopened.read(&one.id).await.unwrap().unwrap().bytes, PNG_A);
    }

    #[tokio::test]
    async fn local_paths_outside_allowlist_and_oversized_files_are_rejected() {
        let allowed = TestDirectory::new("allowed-root");
        let outside = TestDirectory::new("outside-root");
        let image = outside.0.join("outside.png");
        tokio::fs::write(&image, PNG_A).await.unwrap();
        let mut limits = config(&allowed.0);
        limits.max_artwork_bytes = 8;
        let cache = ArtworkCache::new(allowed.0.join("cache"), limits)
            .await
            .unwrap();
        assert!(matches!(
            cache.resolve(image.to_str().unwrap()).await,
            Err(ArtworkError::LocalPathDenied)
        ));

        let allowed_image = allowed.0.join("large.png");
        tokio::fs::write(&allowed_image, PNG_A).await.unwrap();
        assert!(matches!(
            cache.resolve(allowed_image.to_str().unwrap()).await,
            Err(ArtworkError::TooLarge)
        ));
    }

    #[tokio::test]
    async fn cache_eviction_enforces_entry_and_byte_limits() {
        let root = TestDirectory::new("eviction");
        let first = root.0.join("first.png");
        let second = root.0.join("second.png");
        tokio::fs::write(&first, PNG_A).await.unwrap();
        tokio::fs::write(&second, PNG_B).await.unwrap();
        let mut limits = config(&root.0);
        limits.max_entries = 1;
        limits.max_cache_bytes = PNG_B.len();
        let cache = ArtworkCache::new(root.0.join("cache"), limits)
            .await
            .unwrap();
        let first_ref = cache.resolve(first.to_str().unwrap()).await.unwrap();
        let second_ref = cache.resolve(second.to_str().unwrap()).await.unwrap();
        assert_ne!(first_ref.id, second_ref.id);
        assert_eq!(cache.stats().await.entries, 1);
        assert!(cache.read(&first_ref.id).await.unwrap().is_none());
        assert!(cache.read(&second_ref.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn invalid_images_use_fallback_and_remote_urls_reject_local_targets() {
        let root = TestDirectory::new("validation");
        let invalid = root.0.join("not-an-image.bin");
        tokio::fs::write(&invalid, b"not an image").await.unwrap();
        let cache = ArtworkCache::new(root.0.join("cache"), config(&root.0))
            .await
            .unwrap();
        assert!(matches!(
            cache.resolve(invalid.to_str().unwrap()).await,
            Err(ArtworkError::InvalidImage)
        ));
        assert_eq!(
            cache
                .resolve_or_fallback(Some(invalid.to_str().unwrap()))
                .await
                .id,
            FALLBACK_ID
        );
        let fallback = cache.resolve_or_fallback(None).await;
        assert_eq!(fallback.id, FALLBACK_ID);
        assert_eq!(
            cache.read(FALLBACK_ID).await.unwrap().unwrap().mime_type,
            "image/svg+xml"
        );
        assert_eq!(
            cache
                .resolve_or_fallback(Some("http://127.0.0.1/cover.png"))
                .await
                .id,
            FALLBACK_ID
        );

        assert!(validate_remote_uri("https://example.com/cover.png").is_ok());
        for uri in [
            "http://127.0.0.1/secret.png",
            "http://192.168.1.1/cover.png",
            "http://localhost/cover.png",
            "https://example.com:8443/cover.png",
            "file:///etc/passwd",
            "https://user:password@example.com/cover.png",
        ] {
            assert!(validate_remote_uri(uri).is_err(), "accepted {uri}");
        }
        assert!(!is_public_address("::1".parse().unwrap()));
        assert!(!is_public_address("fd00::1".parse().unwrap()));
    }
}
