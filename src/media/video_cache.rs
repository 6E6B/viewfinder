//! Completed speculative downloads only. Incomplete/oversized clips keep streaming
//! through playbin; a growing MP4 must never be handed to a normal file source.
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;

const MAX_CLIP_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_CLIPS: usize = 3;

pub(super) struct Clip {
    file: tempfile::TempPath,
    bytes: u64,
}
impl Clip {
    pub fn uri(&self) -> String {
        reqwest::Url::from_file_path(&self.file)
            .expect("absolute temporary path")
            .into()
    }
}

#[derive(Default)]
pub(super) struct VideoCache(VecDeque<(String, Arc<Clip>)>);
impl VideoCache {
    pub fn get(&mut self, url: &str) -> Option<Arc<Clip>> {
        let index = self.0.iter().position(|(key, _)| key == url)?;
        let entry = self.0.remove(index)?;
        let clip = entry.1.clone();
        self.0.push_back(entry);
        Some(clip)
    }
    pub fn insert(&mut self, url: String, clip: Arc<Clip>) {
        self.0.retain(|(key, _)| key != &url);
        self.0.push_back((url, clip));
        while self.0.len() > MAX_CLIPS
            || self.0.iter().map(|(_, clip)| clip.bytes).sum::<u64>() > MAX_CACHE_BYTES
        {
            self.0.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn remove(&mut self, url: &str) {
        self.0.retain(|(key, _)| key != url);
    }
}

pub(super) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("video preload HTTP client")
}

pub(super) async fn download(http: reqwest::Client, url: String) -> Result<Arc<Clip>, String> {
    super::validate_media_url(&url)?;
    let response = http.get(url).send().await.map_err(|_| "download")?;
    store_response(response, MAX_CLIP_BYTES).await
}

async fn store_response(mut response: reqwest::Response, limit: u64) -> Result<Arc<Clip>, String> {
    // Reject redirects and partial responses as well as HTTP errors.
    if response.status() != reqwest::StatusCode::OK {
        return Err("unavailable".into());
    }
    let expected = response.content_length();
    if expected.is_some_and(|bytes| bytes > limit) {
        return Err("clip too large to preload".into());
    }
    let (file, path) = tokio::task::spawn_blocking(|| {
        tempfile::Builder::new()
            .prefix("viewfinder-reel-")
            .suffix(".mp4")
            .tempfile()
            .map(|file| file.into_parts())
    })
    .await
    .map_err(|_| "temporary file")?
    .map_err(|_| "temporary file")?;
    let mut file = tokio::fs::File::from_std(file);
    let mut bytes = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|_| "download")? {
        bytes += chunk.len() as u64;
        if bytes > limit {
            return Err("clip too large to preload".into());
        }
        file.write_all(&chunk).await.map_err(|_| "temporary file")?;
    }
    if bytes == 0 || expected.is_some_and(|expected| bytes != expected) {
        return Err("incomplete clip".into());
    }
    file.flush().await.map_err(|_| "temporary file")?;
    Ok(Arc::new(Clip { file: path, bytes }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Exercise real response framing, including unknown-length and truncated bodies.
    async fn response(headers: &str, body: &[u8]) -> reqwest::Response {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let bytes = [headers.as_bytes(), body].concat();
        std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(&bytes);
        });
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("http://{address}/clip"))
            .send()
            .await
            .unwrap()
    }

    #[test]
    fn completed_download_is_readable_and_removed_on_release() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let response = response("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n", b"clip").await;
            let clip = store_response(response, 4).await.unwrap();
            let path = clip.file.to_path_buf();
            assert_eq!(std::fs::read(&path).unwrap(), b"clip");
            assert_eq!(
                reqwest::Url::parse(&clip.uri())
                    .unwrap()
                    .to_file_path()
                    .unwrap(),
                path
            );
            drop(clip);
            assert!(!path.exists());
        });
    }

    #[test]
    fn preload_rejects_oversized_partial_empty_and_truncated_downloads() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            for (headers, body) in [
                (
                    "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n",
                    b"large".as_slice(),
                ),
                (
                    "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n",
                    b"large".as_slice(),
                ),
                (
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\n\r\n",
                    b"clip".as_slice(),
                ),
                (
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",
                    b"cut".as_slice(),
                ),
                (
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
                    b"".as_slice(),
                ),
                (
                    "HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n",
                    b"".as_slice(),
                ),
            ] {
                assert!(
                    store_response(response(headers, body).await, 4)
                        .await
                        .is_err()
                );
            }
        });
    }

    fn clip(bytes: u64) -> Arc<Clip> {
        Arc::new(Clip {
            file: tempfile::NamedTempFile::new().unwrap().into_temp_path(),
            bytes,
        })
    }

    #[test]
    fn cache_evicts_old_clips_but_keeps_active_files_alive() {
        let mut cache = VideoCache::default();
        let active = clip(1);
        let path = active.file.to_path_buf();
        cache.insert("active".into(), active.clone());
        for key in ["b", "c", "d"] {
            cache.insert(key.into(), clip(1));
        }
        assert!(cache.get("active").is_none());
        assert!(path.exists());
        drop(active);
        assert!(!path.exists());
        cache.get("b"); // Most recently used survives the next insertion.
        cache.insert("e".into(), clip(1));
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_none());
        cache.clear();
        cache.insert("large-a".into(), clip(MAX_CACHE_BYTES));
        cache.insert("large-b".into(), clip(1));
        assert!(cache.get("large-a").is_none());
        cache.remove("large-b");
        assert!(cache.get("large-b").is_none());
    }

    #[test]
    #[ignore = "requires ffmpeg, H.264 encoder and GStreamer decoder plugins; no display or Viewfinder needed"]
    fn preloaded_mp4_plays_without_a_network_source() {
        use gstreamer::{self as gst, prelude::*};
        let fixture = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
        assert!(
            std::process::Command::new("ffmpeg")
                .args([
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=32x32:d=0.2",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-movflags",
                    "+faststart",
                    "-y",
                ])
                .arg(fixture.path())
                .status()
                .unwrap()
                .success()
        );
        let bytes = std::fs::read(fixture.path()).unwrap();
        let clip = tokio::runtime::Runtime::new().unwrap().block_on(async {
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bytes.len());
            store_response(response(&headers, &bytes).await, MAX_CLIP_BYTES)
                .await
                .unwrap()
        });
        gst::init().unwrap();
        let sink = gst::ElementFactory::make("fakesink").build().unwrap();
        let player = gst::ElementFactory::make("playbin")
            .property("uri", clip.uri())
            .property("video-sink", sink)
            .build()
            .unwrap();
        player.set_state(gst::State::Playing).unwrap();
        let message = player.bus().unwrap().timed_pop_filtered(
            gst::ClockTime::from_seconds(5),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
        player.set_state(gst::State::Null).unwrap();
        assert!(
            matches!(
                message.as_ref().map(|m| m.view()),
                Some(gst::MessageView::Eos(_))
            ),
            "{message:?}"
        );
    }
}
