//! Listing reads that clients share: one computation answers every request it is fresh enough for.

use crate::state;
use priory_runtime::http::{Reply, Request};
use rusqlite::Connection;
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a body of the same revision stands: some of it follows the clock or the disk.
const KEPT: Duration = Duration::from_secs(2);
/// The paths with a kept body. A client may send any query, so past this they are all forgotten.
const PATHS: usize = 64;

/// A computed body, with what it was computed from.
struct Held {
    began: Instant,
    revision: i64,
    /// What the factory served, which a reload replaces.
    factory: Arc<dyn Any + Send + Sync>,
    reply: Reply,
}

type Place = Arc<Mutex<Option<Held>>>;

#[derive(Default)]
pub(crate) struct Shared(Mutex<HashMap<String, Place>>);

impl Shared {
    /// The reply to `request`: the kept one when it was computed after the request arrived, or at
    /// the revision of now and lately, else what `read` computes. Requests for one path wait for
    /// the same computation. Only a `200` is kept.
    pub(crate) fn read<F: Any + Send + Sync>(
        &self,
        db: &Connection,
        factory: &Arc<F>,
        request: &Request,
        read: impl FnOnce() -> Reply,
    ) -> Reply {
        let place = {
            let mut places = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            // A body no request may take any more is dropped, so a quiet path holds no memory.
            places.retain(|_, place| {
                Arc::strong_count(place) > 1
                    || place.try_lock().map_or(true, |held| {
                        held.as_ref()
                            .is_some_and(|held| held.began.elapsed() < KEPT)
                    })
            });
            if places.len() >= PATHS && !places.contains_key(request.path) {
                places.clear();
            }
            places.entry(request.path.to_owned()).or_default().clone()
        };
        let mut held = place.lock().unwrap_or_else(PoisonError::into_inner);
        let began = Instant::now();
        let revision = state::revision(db).ok();
        let factory: Arc<dyn Any + Send + Sync> = factory.clone();
        let fresh = held.as_ref().filter(|held| {
            Arc::ptr_eq(&held.factory, &factory)
                && (held.began >= request.arrived
                    || (Some(held.revision) == revision && held.began.elapsed() < KEPT))
        });
        if let Some(held) = fresh {
            return unchanged(request, copy(&held.reply));
        }
        let reply = read();
        *held = revision
            .filter(|_| reply.status == 200)
            .map(|revision| Held {
                began,
                revision,
                factory,
                reply: copy(&reply),
            });
        unchanged(request, reply)
    }
}

/// The reply again, on the same bytes.
fn copy(reply: &Reply) -> Reply {
    Reply {
        status: reply.status,
        kind: reply.kind,
        headers: reply.headers.clone(),
        body: Arc::clone(&reply.body),
    }
}

/// A listing's `200`. Its `ETag` leaves out the revision, which moves when other rows change.
pub(crate) fn tagged(mut body: Value) -> Reply {
    let revision = body
        .as_object_mut()
        .and_then(|body| body.remove("revision"));
    let Some(revision) = revision.and_then(|revision| revision.as_i64()) else {
        return Reply::json(200, body.to_string());
    };
    let rest = body.to_string();
    let tag = crate::keyboard::hash(&rest);
    let text = match rest.as_str() {
        "{}" => format!("{{\"revision\":{revision}}}"),
        rest => format!("{{\"revision\":{revision},{}", &rest[1..]),
    };
    Reply {
        headers: format!("ETag: \"{tag}\"\r\nPriory-Revision: {revision}\r\n").into(),
        ..Reply::json(200, text)
    }
}

/// `304` with the same headers when `request` holds the body already.
fn unchanged(request: &Request, reply: Reply) -> Reply {
    let tag = request
        .header("if-none-match")
        .and_then(|tag| std::str::from_utf8(tag).ok());
    match tag {
        Some(tag) if reply.headers.starts_with(&format!("ETag: {tag}\r\n")) => Reply {
            status: 304,
            body: Arc::new([]),
            ..reply
        },
        _ => reply,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn request(arrived: Instant) -> Request<'static> {
        Request::get("/api/asks", arrived)
    }

    fn body(reply: &Reply) -> String {
        String::from_utf8(reply.body.to_vec()).unwrap()
    }

    #[test]
    fn requests_that_arrive_together_share_one_computation() {
        let (shared, computed) = (Shared::default(), AtomicUsize::new(0));
        let factory = Arc::new(());
        let start = Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let db = state::memory();
                    let arrived = Instant::now();
                    start.wait();
                    let reply = shared.read(&db, &factory, &request(arrived), || {
                        computed.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Reply::json(200, "{}".to_owned())
                    });
                    assert_eq!(reply.status, 200);
                });
            }
        });
        assert_eq!(computed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_kept_body_ends_with_its_revision_its_factory_and_its_time_and_a_failure_is_never_kept() {
        let db = state::memory();
        let shared = Shared::default();
        let (one, other) = (Arc::new(1), Arc::new(2));
        let read = |factory: &Arc<i32>, text: &str, status: u16| {
            let reply = || Reply::json(status, text.to_owned());
            body(&shared.read(&db, factory, &request(Instant::now()), reply))
        };
        assert_eq!(read(&one, "failed", 500), "failed");
        assert_eq!(read(&one, "a", 200), "a");
        assert_eq!(read(&one, "b", 200), "a");
        db.execute("UPDATE revision SET value = value + 1", [])
            .unwrap();
        assert_eq!(read(&one, "b", 200), "b");
        assert_eq!(read(&other, "c", 200), "c");
        std::thread::sleep(KEPT);
        assert_eq!(read(&other, "d", 200), "d");
    }

    #[test]
    fn a_body_past_its_time_is_dropped_by_the_next_read_of_any_path() {
        let db = state::memory();
        let shared = Shared::default();
        let factory = Arc::new(());
        let reply = || Reply::json(200, "{}".to_owned());
        shared.read(&db, &factory, &request(Instant::now()), reply);
        std::thread::sleep(KEPT);
        let other = Request::get("/api/desk", Instant::now());
        shared.read(&db, &factory, &other, reply);
        let places = shared.0.lock().unwrap();
        assert_eq!(places.keys().collect::<Vec<_>>(), ["/api/desk"]);
    }

    #[test]
    fn a_body_keeps_its_revision_and_its_tag_leaves_it_out() {
        let tag = |reply: &Reply| reply.headers.lines().next().map(str::to_owned);
        let one = tagged(serde_json::json!({ "revision": 1, "asks": [{ "revision": 1 }] }));
        let two = tagged(serde_json::json!({ "revision": 2, "asks": [{ "revision": 1 }] }));
        let moved = tagged(serde_json::json!({ "revision": 2, "asks": [{ "revision": 2 }] }));
        assert_eq!(tag(&one), tag(&two));
        assert_ne!(tag(&two), tag(&moved));
        let parsed: Value = serde_json::from_str(&body(&two)).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({ "revision": 2, "asks": [{ "revision": 1 }] })
        );
        assert_eq!(
            body(&tagged(serde_json::json!({ "revision": 3 }))),
            r#"{"revision":3}"#
        );
    }
}
