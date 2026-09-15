use crate::{
    domain::{Page, Route},
    viewfinder::{Client, Error},
};
use std::{collections::HashSet, future::Future, sync::Arc};

/// Tokio owns all I/O. Only completion callbacks execute on GLib.
/// Dropping a Task cancels the future, including an in-flight HTTP request.
pub struct Task(
    Option<tokio::task::AbortHandle>,
    Arc<std::sync::atomic::AtomicBool>,
);
impl Drop for Task {
    fn drop(&mut self) {
        self.1.store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}
impl Task {
    /// Suppress the completion callback but let the future finish, so shared
    /// caches still receive results the abandoned widget triggered.
    pub fn detach(mut self) {
        self.0 = None;
    }
}

pub fn background<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
    done: impl FnOnce(T) + 'static,
) -> Task {
    let (send, recv) = async_channel::bounded(1);
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_cancelled = cancelled.clone();
    let task = tokio::spawn(async move {
        let result = future.await;
        let _ = send.send(result).await;
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(result) = recv.recv().await
            && !callback_cancelled.load(std::sync::atomic::Ordering::Acquire)
        {
            done(result);
        }
    });
    Task(Some(task.abort_handle()), cancelled)
}

#[derive(Default)]
pub struct Pagination {
    pub cursor: Option<String>,
    pub loading: bool,
    pub started: bool,
    pub exhausted: bool,
    pub generation: u64,
    seen: HashSet<String>,
}

impl Pagination {
    pub fn begin(&mut self) -> Option<(u64, Option<String>)> {
        if self.loading || self.exhausted {
            return None;
        }
        self.loading = true;
        Some((self.generation, self.cursor.clone()))
    }
    pub fn finish(&mut self, generation: u64, page: &mut Page) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        self.started = true;
        self.exhausted = page.next.is_none() || page.next == self.cursor;
        self.cursor = page.next.clone();
        page.items.retain(|i| self.seen.insert(i.id().to_owned()));
        true
    }
    pub fn reset(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::default()
        };
    }
}

pub async fn load(
    client: Arc<Client>,
    route: Route,
    cursor: Option<String>,
) -> Result<Page, Error> {
    client.load(route, cursor).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Item, User};
    #[test]
    fn large_collection_pages_keep_existing_items_and_stop_repeated_cursor() {
        let mut pagination = Pagination::default();
        let mut count = 0;
        for index in 0..100 {
            let (generation, _) = pagination.begin().unwrap();
            let mut page = Page::complete(
                (0..21)
                    .map(|offset| {
                        Item::User(User {
                            id: (index * 20 + offset).to_string(),
                            ..User::default()
                        })
                    })
                    .collect(),
            );
            page.next = Some(index.to_string());
            assert!(pagination.finish(generation, &mut page));
            count += page.items.len();
        }
        assert_eq!(count, 2001);
        let (generation, cursor) = pagination.begin().unwrap();
        let mut page = Page::complete(vec![]);
        page.next = cursor;
        assert!(pagination.finish(generation, &mut page));
        assert!(pagination.begin().is_none());
    }

    #[test]
    fn dropping_task_suppresses_already_queued_completion() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _runtime = runtime.enter();
        let context = gtk::glib::MainContext::new();
        context
            .with_thread_default(|| {
                let called = std::rc::Rc::new(std::cell::Cell::new(false));
                let marker = called.clone();
                let (ready, signal) = std::sync::mpsc::channel();
                let task = background(
                    async move {
                        ready.send(()).unwrap();
                    },
                    move |_| marker.set(true),
                );
                signal
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                drop(task);
                while context.pending() {
                    context.iteration(false);
                }
                assert!(!called.get());
            })
            .unwrap();
    }
    #[test]
    fn pagination_rejects_stale_and_deduplicates() {
        let mut p = Pagination::default();
        let (old, _) = p.begin().unwrap();
        assert!(p.begin().is_none());
        p.reset();
        let mut page = Page::complete(vec![]);
        assert!(!p.finish(old, &mut page));
        let (current, _) = p.begin().unwrap();
        page.items = vec![
            Item::User(User {
                id: "1".into(),
                ..User::default()
            });
            2
        ];
        assert!(p.finish(current, &mut page));
        assert_eq!(page.items.len(), 1);
        assert!(p.begin().is_none());
    }
}
