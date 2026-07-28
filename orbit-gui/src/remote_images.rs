use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use futures_util::StreamExt as _;
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest as _, Sha256};

const USER_AGENT: &str = "orbit-gui/0.1.0 (https://github.com/water2004/orbit)";
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_DIMENSION: u32 = 4096;
const NORMALIZED_DIMENSION: u32 = 128;

struct ImageResult {
    url: String,
    path: Result<PathBuf, String>,
}

/// Downloads presentation-only images outside GPUI's Windows work-item callback,
/// validates decoder resource limits, and normalizes them to small local PNGs.
/// The renderer consequently never receives an untrusted remote image resource.
pub(crate) struct RemoteImageBridge {
    requests: Sender<String>,
    results: Receiver<ImageResult>,
    pending: HashSet<String>,
    paths: HashMap<String, PathBuf>,
    failed: HashSet<String>,
    cache_dir: Option<PathBuf>,
}

impl RemoteImageBridge {
    pub(crate) fn new() -> Self {
        let cache_dir = directories::ProjectDirs::from("dev", "Orbit", "Orbit GUI")
            .map(|dirs| dirs.cache_dir().join("images"));
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker_cache = cache_dir.clone();
        std::thread::Builder::new()
            .name("orbit-image-cache".to_string())
            .spawn(move || run_worker(request_rx, result_tx, worker_cache))
            .expect("image cache worker thread can be created");
        Self {
            requests: request_tx,
            results: result_rx,
            pending: HashSet::new(),
            paths: HashMap::new(),
            failed: HashSet::new(),
            cache_dir,
        }
    }

    pub(crate) fn request(&mut self, url: &str) {
        if self.paths.contains_key(url) || self.pending.contains(url) || self.failed.contains(url) {
            return;
        }
        if let Some(path) = self.cache_path(url)
            && is_normalized_png(&path)
        {
            self.paths.insert(url.to_string(), path);
            return;
        }
        self.pending.insert(url.to_string());
        if self.requests.send(url.to_string()).is_err() {
            self.pending.remove(url);
            self.failed.insert(url.to_string());
        }
    }

    pub(crate) fn path(&self, url: &str) -> Option<&Path> {
        self.paths.get(url).map(PathBuf::as_path)
    }

    /// Applies all completed downloads and reports whether the visible state changed.
    pub(crate) fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.results.try_recv() {
            self.pending.remove(&result.url);
            match result.path {
                Ok(path) => {
                    self.paths.insert(result.url, path);
                }
                Err(_) => {
                    self.failed.insert(result.url);
                }
            }
            changed = true;
        }
        changed
    }

    fn cache_path(&self, url: &str) -> Option<PathBuf> {
        self.cache_dir
            .as_ref()
            .map(|directory| directory.join(cache_filename(url)))
    }
}

fn run_worker(
    requests: Receiver<String>,
    results: Sender<ImageResult>,
    cache_dir: Option<PathBuf>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("orbit-image-download")
        .build()
    else {
        return;
    };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(20))
        .build()
    else {
        return;
    };

    while let Ok(url) = requests.recv() {
        let results = results.clone();
        let client = client.clone();
        let cache_dir = cache_dir.clone();
        runtime.spawn(async move {
            let path = fetch_and_normalize(&client, &url, cache_dir.as_deref())
                .await
                .map_err(|error| error.to_string());
            let _ = results.send(ImageResult { url, path });
        });
    }
}

async fn fetch_and_normalize(
    client: &reqwest::Client,
    url: &str,
    cache_dir: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" {
        anyhow::bail!("remote presentation images must use HTTPS");
    }
    let cache_dir = cache_dir.ok_or_else(|| anyhow::anyhow!("image cache is unavailable"))?;
    let response = client.get(parsed).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        anyhow::bail!("remote image exceeds the 8 MiB presentation limit");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            anyhow::bail!("remote image exceeds the 8 MiB presentation limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let normalized = normalize_png(&bytes)?;
    std::fs::create_dir_all(cache_dir)?;
    let destination = cache_dir.join(cache_filename(url));
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, normalized)?;
    if destination.exists() {
        std::fs::remove_file(&destination)?;
    }
    std::fs::rename(temporary, &destination)?;
    Ok(destination)
}

fn normalize_png(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()?
        .thumbnail(NORMALIZED_DIMENSION, NORMALIZED_DIMENSION);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn is_normalized_png(path: &Path) -> bool {
    let Ok(reader) = ImageReader::open(path).and_then(ImageReader::with_guessed_format) else {
        return false;
    };
    reader.into_dimensions().is_ok_and(|(width, height)| {
        width <= NORMALIZED_DIMENSION && height <= NORMALIZED_DIMENSION
    })
}

fn cache_filename(url: &str) -> String {
    format!("{:x}.png", Sha256::digest(url.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_remote_images_to_bounded_png() {
        let source = image::DynamicImage::new_rgba8(512, 256);
        let mut bytes = Cursor::new(Vec::new());
        source.write_to(&mut bytes, ImageFormat::Png).unwrap();

        let normalized = normalize_png(bytes.get_ref()).unwrap();
        assert_eq!(image::guess_format(&normalized).unwrap(), ImageFormat::Png);
        let decoded = image::load_from_memory(&normalized).unwrap();
        assert!(decoded.width() <= NORMALIZED_DIMENSION);
        assert!(decoded.height() <= NORMALIZED_DIMENSION);
    }

    #[test]
    fn cache_key_does_not_expose_or_depend_on_url_path_syntax() {
        let filename = cache_filename("https://cdn.example.invalid/a/icon.png?size=512");
        assert_eq!(filename.len(), 68);
        assert!(filename.ends_with(".png"));
        assert!(!filename.contains("icon"));
    }
}
