use crate::app::{self, Task};
mod video_cache;
use gstreamer::{self as gst, prelude::*};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Pixels {
    pub data: glib::Bytes,
    pub width: i32,
    pub height: i32,
    pub stride: usize,
    pub alpha: bool,
}
impl Pixels {
    fn texture(&self) -> gdk::MemoryTexture {
        gdk::MemoryTexture::new(
            self.width,
            self.height,
            if self.alpha {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            },
            &self.data,
            self.stride,
        )
    }
}
type ImageEntries = VecDeque<(String, Pixels)>;
pub struct Images {
    http: reqwest::Client,
    cache: Mutex<ImageEntries>,
    slots: tokio::sync::Semaphore,
    avatar_slots: tokio::sync::Semaphore,
    requests: Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
}
impl Images {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("media HTTP client"),
            cache: Mutex::new(VecDeque::new()),
            slots: tokio::sync::Semaphore::new(6),
            avatar_slots: tokio::sync::Semaphore::new(4),
            requests: Mutex::new(HashMap::new()),
        })
    }
    #[cfg(test)]
    pub fn review_seed(&self) {
        let mut portrait = Vec::with_capacity(360 * 640 * 3);
        for y in 0..640 {
            for x in 0..360 {
                portrait.extend_from_slice(if !(8..352).contains(&x) || !(8..632).contains(&y) {
                    &[240, 195, 90]
                } else if y < 320 {
                    &[50, 100, 135]
                } else {
                    &[30, 65, 85]
                });
            }
        }
        self.cache.lock().unwrap().push_back((
            "1600:fixture://portrait".into(),
            Pixels {
                data: glib::Bytes::from_owned(portrait),
                width: 360,
                height: 640,
                stride: 1080,
                alpha: false,
            },
        ));
        let pixels = if let Ok(p) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(
            "/usr/share/backgrounds/gnome/vnc-l.png",
            800,
            600,
            false,
        ) {
            Pixels {
                data: p.read_pixel_bytes(),
                width: p.width(),
                height: p.height(),
                stride: p.rowstride() as usize,
                alpha: p.has_alpha(),
            }
        } else {
            Pixels {
                data: glib::Bytes::from_owned(vec![100u8; 800 * 600 * 3]),
                width: 800,
                height: 600,
                stride: 2400,
                alpha: false,
            }
        };
        for size in [64, 80, 128, 400, 1080, 1600] {
            self.cache
                .lock()
                .unwrap()
                .push_back((format!("{size}:fixture://image"), pixels.clone()));
        }
    }
    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
    }
    fn cached(&self, key: &str) -> Option<Pixels> {
        let mut cache = self.cache.lock().unwrap();
        let index = cache.iter().position(|(k, _)| k == key)?;
        let entry = cache.remove(index)?;
        let pixels = entry.1.clone();
        cache.push_back(entry);
        Some(pixels)
    }
    fn request_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut requests = self.requests.lock().unwrap();
        requests.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = requests.get(key).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        requests.insert(key.to_owned(), Arc::downgrade(&lock));
        lock
    }
    async fn get(&self, url: String, size: i32) -> Result<Pixels, String> {
        let started = Instant::now();
        let key = format!("{size}:{url}");
        if let Some(pixels) = self.cached(&key) {
            return Ok(pixels);
        }
        // Wait for an identical request without consuming a download slot.
        // Cancellation releases the lock so another visible widget can retry.
        let request = self.request_lock(&key);
        let _request = request.lock().await;
        if let Some(pixels) = self.cached(&key) {
            return Ok(pixels);
        }
        // Avatars must not queue behind large post images.
        let slots = if size <= 128 {
            &self.avatar_slots
        } else {
            &self.slots
        };
        let _slot = slots.acquire().await.map_err(|_| "cancelled")?;
        validate_media_url(&url)?;
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| "download")?
            .error_for_status()
            .map_err(|_| "unavailable")?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "download")? {
            if bytes.len() + chunk.len() > 20 * 1024 * 1024 {
                return Err("image too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let downloaded_bytes = bytes.len();
        let pixels = tokio::task::spawn_blocking(move || {
            // GdkPixbuf objects are created, decoded and destroyed on this worker.
            // Only immutable pixel bytes cross into GTK.
            let loader = gtk::gdk_pixbuf::PixbufLoader::new();
            loader.connect_size_prepared(move |loader, w, h| {
                let ratio = (size as f64 / w.max(h).max(1) as f64).min(1.0);
                loader.set_size(
                    (w as f64 * ratio).max(1.0) as i32,
                    (h as f64 * ratio).max(1.0) as i32,
                );
            });
            loader.write(&bytes).map_err(|_| "decode")?;
            loader.close().map_err(|_| "decode")?;
            let p = loader.pixbuf().ok_or("decode")?;
            Ok::<_, &str>(Pixels {
                data: p.read_pixel_bytes(),
                width: p.width(),
                height: p.height(),
                stride: p.rowstride() as usize,
                alpha: p.has_alpha(),
            })
        })
        .await
        .map_err(|_| "decode")?
        .map_err(str::to_owned)?;
        tracing::debug!(
            size,
            downloaded_bytes,
            elapsed_ms = started.elapsed().as_millis(),
            "Image ready"
        );
        let mut cache = self.cache.lock().unwrap();
        cache.push_back((key, pixels.clone()));
        let mut total: usize = cache.iter().map(|(_, p)| p.data.len()).sum();
        while total > 64 * 1024 * 1024 {
            if let Some((_, p)) = cache.pop_front() {
                total -= p.data.len();
            } else {
                break;
            }
        }
        Ok(pixels)
    }
    /// Fill the cache in the background. `size` must match the eventual
    /// `get()` call; identical keys share the in-flight request lock, so a
    /// row that maps mid-download joins this request rather than repeating it.
    pub fn prefetch(self: &Arc<Self>, url: Option<String>, size: i32) {
        let Some(url) = url else { return };
        let images = Arc::clone(self);
        tokio::spawn(async move {
            let _ = images.get(url, size).await;
        });
    }
    /// Cache-fill matching the `avatar()` request key.
    pub fn prefetch_avatar(self: &Arc<Self>, user: &crate::domain::User, size: i32) {
        self.prefetch(user.avatar.clone(), size * 2);
    }
    /// Cache-fill matching the `preview()`/`picture()` request key.
    pub fn prefetch_media(self: &Arc<Self>, media: &crate::domain::Media, size: i32) {
        self.prefetch(preview_url(media, size), size);
    }
}
pub fn validate_media_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "invalid URL")?;
    let host = parsed.host_str().ok_or("invalid host")?;
    if parsed.scheme() != "https"
        || ![
            "cdninstagram.com",
            "fbcdn.net",
            "fbsbx.com",
            "instagram.com",
            "giphy.com",
            "tenor.com",
        ]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return Err("untrusted media URL".into());
    }
    Ok(())
}

pub fn picture(images: Arc<Images>, url: Option<String>, size: i32) -> gtk::Picture {
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .hexpand(true)
        .build();
    picture.update_property(&[gtk::accessible::Property::Label("Post image")]);
    let task: Rc<RefCell<Option<Task>>> = Rc::default();
    let generation = Rc::new(Cell::new(0u64));
    picture.connect_map(glib::clone!(
        #[strong]
        task,
        #[strong]
        generation,
        move |p| {
            let Some(url) = url.clone() else {
                p.set_tooltip_text(Some("Image unavailable"));
                return;
            };
            let images = images.clone();
            let ticket = generation.get();
            *task.borrow_mut() = Some(app::background(
                async move { images.get(url, size).await },
                glib::clone!(
                    #[weak]
                    p,
                    #[strong]
                    generation,
                    move |result| {
                        if generation.get() != ticket {
                            return;
                        }
                        match result {
                            Ok(pixels) => {
                                p.set_paintable(Some(&pixels.texture()));
                                p.set_tooltip_text(None);
                            }
                            Err(_) => {
                                p.set_tooltip_text(Some("Image unavailable. Refresh to try again."))
                            }
                        }
                    }
                ),
            ));
        }
    ));
    picture.connect_unmap(glib::clone!(
        #[strong]
        task,
        move |p| {
            generation.set(generation.get() + 1);
            // Let an in-flight download land in the shared cache; a remap
            // resolves from cache instead of restarting the request.
            if let Some(task) = task.borrow_mut().take() {
                task.detach();
            }
            p.set_paintable(None::<&gdk::Paintable>);
        }
    ));
    picture
}

/// A reserved media surface with visible failure feedback. The image keeps its
/// intrinsic ratio while the request is pending, so feed rows do not collapse.
pub fn preview(
    images: Arc<Images>,
    media: &crate::domain::Media,
    size: i32,
    cover: bool,
) -> gtk::Overlay {
    let image = picture(images, preview_url(media, size), size);
    image.set_content_fit(if cover {
        gtk::ContentFit::Cover
    } else {
        gtk::ContentFit::Contain
    });
    let placeholder = gdk::Paintable::new_empty(media.width.max(1), media.height.max(1));
    image.set_paintable(Some(&placeholder));
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&image));
    let message = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_start(12)
        .margin_end(12)
        .build();
    message.add_css_class("dim-label");
    message.set_visible(false);
    overlay.add_overlay(&message);
    image.connect_tooltip_text_notify(glib::clone!(
        #[weak]
        message,
        move |image| {
            let error = image.tooltip_text();
            message.set_label(error.as_deref().unwrap_or_default());
            message.set_visible(error.is_some());
        }
    ));
    image.connect_paintable_notify(glib::clone!(
        #[strong]
        placeholder,
        move |image| {
            if image.paintable().is_none() {
                image.set_paintable(Some(&placeholder));
            }
        }
    ));
    overlay
}

fn preview_url(media: &crate::domain::Media, size: i32) -> Option<String> {
    if size <= 400 || media.video.is_some() {
        media.thumbnail.clone().or_else(|| media.image.clone())
    } else {
        media.image.clone().or_else(|| media.thumbnail.clone())
    }
}

pub fn avatar(images: Arc<Images>, user: &crate::domain::User, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(&user.username), true);
    let Some(url) = user.avatar.clone() else {
        return avatar;
    };
    let task: Rc<RefCell<Option<Task>>> = Rc::default();
    avatar.connect_map(glib::clone!(
        #[strong]
        task,
        move |avatar| {
            let images = images.clone();
            let url = url.clone();
            *task.borrow_mut() = Some(app::background(
                async move { images.get(url, size * 2).await },
                glib::clone!(
                    #[weak]
                    avatar,
                    move |result| {
                        if let Ok(pixels) = result {
                            avatar.set_custom_image(Some(&pixels.texture()));
                        }
                    }
                ),
            ));
        }
    ));
    avatar.connect_unmap(move |avatar| {
        if let Some(task) = task.borrow_mut().take() {
            task.detach();
        }
        avatar.set_custom_image(None::<&gdk::Paintable>);
    });
    avatar
}

/// Exactly one pipeline per application. Opening another video releases the old
/// decoder. Preparation, state changes and teardown run on Tokio workers.
pub struct Playback {
    commands: tokio::sync::mpsc::UnboundedSender<Box<dyn FnOnce() + Send>>,
    current: RefCell<Option<gst::Element>>,
    watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    pub muted: Cell<bool>,
    pub failed: Cell<bool>,
    paused: Cell<bool>,
    buffering: Cell<bool>,
    active_picture: glib::WeakRef<gtk::Picture>,
    videos: RefCell<video_cache::VideoCache>,
    video_http: reqwest::Client,
    next_video: RefCell<Option<String>>,
    preload: RefCell<Option<Task>>,
    playing: Cell<bool>,
    active_clip: RefCell<Option<Arc<video_cache::Clip>>>,
}
impl Playback {
    pub fn new() -> Rc<Self> {
        let (commands, mut receive) =
            tokio::sync::mpsc::unbounded_channel::<Box<dyn FnOnce() + Send>>();
        tokio::spawn(async move {
            // Serialize state transitions: an obsolete prepare cannot run after
            // its teardown, and the previous decoder stops before the next starts.
            while let Some(command) = receive.recv().await {
                let _ = tokio::task::spawn_blocking(command).await;
            }
        });
        Rc::new(Self {
            commands,
            current: RefCell::new(None),
            watch: RefCell::new(None),
            muted: Cell::new(false),
            failed: Cell::new(false),
            paused: Cell::new(false),
            buffering: Cell::new(false),
            active_picture: glib::WeakRef::new(),
            videos: RefCell::new(video_cache::VideoCache::default()),
            video_http: video_cache::client(),
            next_video: RefCell::new(None),
            preload: RefCell::new(None),
            playing: Cell::new(false),
            active_clip: RefCell::new(None),
        })
    }
    pub fn preload_next(self: &Rc<Self>, url: Option<String>) {
        if *self.next_video.borrow() != url {
            self.preload.borrow_mut().take();
            self.next_video.replace(url);
        }
        self.schedule_preload();
    }
    fn schedule_preload(self: &Rc<Self>) {
        if !self.playing.get()
            || self.paused.get()
            || self.buffering.get()
            || self.failed.get()
            || self.preload.borrow().is_some()
        {
            return;
        }
        let Some(url) = self.next_video.borrow().clone() else {
            return;
        };
        if self.videos.borrow_mut().get(&url).is_some() {
            return;
        }
        let http = self.video_http.clone();
        let key = url.clone();
        *self.preload.borrow_mut() = Some(app::background(
            async move {
                // Debounce rapid scrolling and give the active stream a head start.
                tokio::time::sleep(Duration::from_millis(800)).await;
                video_cache::download(http, url).await
            },
            glib::clone!(
                #[weak(rename_to = controller)]
                self,
                move |result| {
                    match result {
                        Ok(clip) => {
                            controller.videos.borrow_mut().insert(key, clip);
                            tracing::debug!("Next reel preload complete");
                        }
                        Err(reason) => tracing::debug!(%reason, "Next reel will stream on demand"),
                    }
                }
            ),
        ));
    }
    pub fn clear_cache(&self) {
        self.preload.borrow_mut().take();
        self.next_video.borrow_mut().take();
        self.videos.borrow_mut().clear();
    }
    pub fn owns(&self, picture: &gtk::Picture) -> bool {
        self.active_picture.upgrade().as_ref() == Some(picture)
    }
    /// Stop only when the active video surface sits inside `owner`. GTK maps
    /// the incoming view before unmapping the outgoing one, so an unmap
    /// handler must not kill a pipeline that another widget just started.
    pub fn stop_owned(&self, owner: &impl IsA<gtk::Widget>) {
        if self
            .active_picture
            .upgrade()
            .is_some_and(|picture| picture.is_ancestor(owner))
        {
            self.stop();
        }
    }
    pub fn stop(&self) {
        self.playing.set(false);
        self.preload.borrow_mut().take();
        self.next_video.borrow_mut().take();
        let picture = self.active_picture.upgrade();
        self.active_picture.set(None);
        if let Some(picture) = picture {
            picture.set_paintable(None::<&gdk::Paintable>);
        }
        self.watch.borrow_mut().take();
        let clip = self.active_clip.borrow_mut().take();
        if let Some(pipeline) = self.current.borrow_mut().take() {
            let _ = self.commands.send(Box::new(move || {
                let _ = pipeline.set_state(gst::State::Null);
                // Pin the file until the decoder has released it, even on eviction.
                drop(clip);
            }));
        }
    }
    pub fn pause(&self, paused: bool) {
        self.paused.set(paused);
        if paused {
            self.preload.borrow_mut().take();
        }
        let paused = paused || self.buffering.get();
        if let Some(p) = self.current.borrow().as_ref().cloned() {
            let _ = self.commands.send(Box::new(move || {
                let _ = p.set_state(if paused {
                    gst::State::Paused
                } else {
                    gst::State::Playing
                });
            }));
        }
    }
    pub fn mute(&self, muted: bool) {
        self.muted.set(muted);
        if let Some(p) = self.current.borrow().as_ref().cloned() {
            let _ = self
                .commands
                .send(Box::new(move || p.set_property("mute", muted)));
        }
    }
    pub fn start(
        self: &Rc<Self>,
        url: &str,
        picture: &gtk::Picture,
        status: &gtk::Label,
    ) -> Result<(), String> {
        validate_media_url(url).map_err(|error| {
            tracing::warn!(reason=%error, "Video URL rejected");
            "This video is unavailable.".to_owned()
        })?;
        self.stop();
        self.failed.set(false);
        self.paused.set(false);
        self.buffering.set(false);
        let clip = self.videos.borrow_mut().get(url);
        let uri = clip
            .as_ref()
            .map(|clip| clip.uri())
            .unwrap_or_else(|| url.to_owned());
        tracing::debug!(cached = clip.is_some(), "Opening video");
        let sink = gst::ElementFactory::make("gtk4paintablesink")
            .build()
            .map_err(|_| "Install the GStreamer GTK 4 plugin to play videos.")?;
        let paintable = sink.property::<gdk::Paintable>("paintable");
        picture.set_paintable(Some(&paintable));
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", &uri)
            .property("video-sink", &sink)
            .property("mute", self.muted.get())
            // A short initial queue reduces startup latency for short clips.
            .property("buffer-duration", 1_000_000_000i64)
            .build()
            .map_err(|_| "Video playback is unavailable.")?;
        let p = pipeline.downgrade();
        let started = Instant::now();
        let mut reported_start = false;
        let mut retry_stream = clip.is_some();
        let source_url = url.to_owned();
        let label = status.downgrade();
        let controller = Rc::downgrade(self);
        let guard = pipeline
            .bus()
            .ok_or("Video playback is unavailable.")?
            .add_watch_local(move |_, message| {
                let Some(controller) = controller.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match message.view() {
                    gst::MessageView::Error(_) => {
                        controller.preload.borrow_mut().take();
                        if retry_stream {
                            retry_stream = false;
                            controller.videos.borrow_mut().remove(&source_url);
                            controller.buffering.set(false);
                            controller.playing.set(false);
                            if let Some(p) = p.upgrade() {
                                let url = source_url.clone();
                                let paused = controller.paused.get();
                                let _ = controller.commands.send(Box::new(move || {
                                    let _ = p.set_state(gst::State::Null);
                                    p.set_property("uri", url);
                                    let _ = p.set_state(if paused {
                                        gst::State::Paused
                                    } else {
                                        gst::State::Playing
                                    });
                                }));
                            }
                            return glib::ControlFlow::Continue;
                        }
                        controller.failed.set(true);
                        if let Some(l) = label.upgrade() {
                            l.set_label("Video unavailable. Use Play to retry.");
                        }
                        if let Some(p) = p.upgrade() {
                            let _ = controller.commands.send(Box::new(move || {
                                let _ = p.set_state(gst::State::Null);
                            }));
                        }
                    }
                    gst::MessageView::Buffering(b) => {
                        let buffering = b.percent() < 100;
                        if buffering {
                            controller.preload.borrow_mut().take();
                        }
                        if controller.buffering.replace(buffering) != buffering
                            && let Some(p) = p.upgrade()
                        {
                            let paused = buffering || controller.paused.get();
                            let _ = controller.commands.send(Box::new(move || {
                                let _ = p.set_state(if paused {
                                    gst::State::Paused
                                } else {
                                    gst::State::Playing
                                });
                            }));
                        }
                        if let Some(l) = label.upgrade() {
                            l.set_label(if b.percent() < 100 {
                                "Buffering…"
                            } else {
                                ""
                            });
                        }
                    }
                    gst::MessageView::StateChanged(s) => {
                        if message.src() != p.upgrade().as_ref().map(|p| p.upcast_ref()) {
                            return glib::ControlFlow::Continue;
                        }
                        controller.playing.set(s.current() == gst::State::Playing);
                        if controller.playing.get() {
                            controller.schedule_preload();
                            if let Some(l) = label.upgrade() {
                                l.set_label("");
                            }
                            if !reported_start {
                                reported_start = true;
                                tracing::debug!(
                                    elapsed_ms = started.elapsed().as_millis(),
                                    "Video playback started"
                                );
                            }
                        }
                    }
                    gst::MessageView::Eos(_) => {
                        if let Some(p) = p.upgrade() {
                            let _ = controller.commands.send(Box::new(move || {
                                let _ = p.seek_simple(
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                    gst::ClockTime::ZERO,
                                );
                            }));
                        }
                    }
                    _ => (),
                }
                glib::ControlFlow::Continue
            })
            .map_err(|_| "Video playback is unavailable.")?;
        self.active_picture.set(Some(picture));
        *self.watch.borrow_mut() = Some(guard);
        *self.current.borrow_mut() = Some(pipeline.clone());
        *self.active_clip.borrow_mut() = clip;
        status.set_label("Buffering…");
        let _ = self.commands.send(Box::new(move || {
            let _ = pipeline.set_state(gst::State::Playing);
        }));
        Ok(())
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_choose_thumbnail_for_tiles_and_video_posters() {
        let mut media = crate::domain::Media {
            thumbnail: Some("small".into()),
            image: Some("large".into()),
            video: None,
            width: 1080,
            height: 1920,
        };
        assert_eq!(preview_url(&media, 400).as_deref(), Some("small"));
        assert_eq!(preview_url(&media, 1080).as_deref(), Some("large"));
        media.video = Some("video".into());
        assert_eq!(preview_url(&media, 1080).as_deref(), Some("small"));
        media.thumbnail = None;
        assert_eq!(preview_url(&media, 400).as_deref(), Some("large"));
    }

    #[test]
    fn image_requests_share_locks_and_cancelled_owners_release_them() {
        let images = Images::new();
        let first = images.request_lock("same-image");
        let second = images.request_lock("same-image");
        assert!(Arc::ptr_eq(&first, &second));
        let guard = first.try_lock().unwrap();
        assert!(second.try_lock().is_err());
        assert!(images.request_lock("another-image").try_lock().is_ok());
        drop(guard);
        assert!(second.try_lock().is_ok());
        drop(first);
        drop(second);
        let _next = images.request_lock("next-image");
        assert_eq!(images.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn image_cache_hits_refresh_recency() {
        let images = Images::new();
        let pixels = Pixels {
            data: glib::Bytes::from_owned(vec![0u8; 3]),
            width: 1,
            height: 1,
            stride: 3,
            alpha: false,
        };
        images
            .cache
            .lock()
            .unwrap()
            .extend([("a".into(), pixels.clone()), ("b".into(), pixels)]);
        assert!(images.cached("a").is_some());
        assert_eq!(images.cache.lock().unwrap().back().unwrap().0, "a");
        images.clear();
        assert!(images.cached("a").is_none());
    }
    #[test]
    fn media_hosts_are_validated_without_network() {
        assert!(validate_media_url("https://scontent.cdninstagram.com/image.jpg").is_ok());
        assert!(validate_media_url("https://media.giphy.com/media/example/giphy.mp4").is_ok());
        assert!(validate_media_url("https://cdn.fbsbx.com/v/t59.2708-21/sticker.webp").is_ok());
        for url in [
            "https://giphy.com.evil.invalid/a",
            "http://scontent.cdninstagram.com/a",
            "https://cdninstagram.com.evil.invalid/a",
            "file:///tmp/a",
            "https://localhost/a",
            "https://notinstagram.com/a",
        ] {
            assert!(validate_media_url(url).is_err());
        }
    }
}
