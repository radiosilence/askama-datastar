//! Answers in server-sent events: a few events at once, or a live stream.
//!
//! Both take anything that becomes an axum SSE [`Event`]: the SDK's patches
//! and signals, or an `Event` written by hand. Neither is stored by a cache,
//! since what they carry is the page as one person sees it.

use std::convert::Infallible;
use std::time::Duration;

use axum::http::{HeaderValue, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{Stream, StreamExt as _};

/// `events` as one answer, which ends once the last is sent; none at all
/// answers without changing the page.
///
/// Each event is axum's [`Event`], which a patch and a signal update each
/// convert into, so one answer can carry both:
///
/// ```
/// use askama_datastar::{Html, PatchSignals, events};
///
/// let response = events([
///     Html::trusted(r#"<p id="flash">Saved.</p>"#).patch().into(),
///     PatchSignals::new(r#"{"name":""}"#).into(),
/// ]);
/// assert_eq!(response.headers()["content-type"], "text/event-stream");
///
/// let nothing = events(Vec::new());
/// ```
#[must_use]
pub fn events(events: impl IntoIterator<Item = Event>) -> Response {
    // Collected, since the body outlives the handler and the iterator may
    // borrow from it.
    let events: Vec<Event> = events.into_iter().collect();
    unstored(Sse::new(futures_util::stream::iter(
        events.into_iter().map(Ok::<_, Infallible>),
    )))
}

/// How long a live stream goes quiet before it sends a heartbeat.
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(15);

/// The heartbeat's event name. Datastar dispatches every event whose name
/// starts with `datastar` as a `datastar-fetch` event on the document, which is
/// what lets a page hear it, and runs nothing for a name it has no watcher for.
pub(crate) const HEARTBEAT_EVENT: &str = "datastar-heartbeat";

/// `events` as they come, for as long as the stream runs.
///
/// After 15 seconds without an event, a heartbeat goes down the connection:
/// `event: datastar-heartbeat`, which Datastar dispatches and otherwise
/// ignores. It keeps a proxy that closes a quiet connection from closing this
/// one, and it gives the page something to hear while nothing changes, so
/// that silence means the stream is gone; [`reopen`](crate::reopen) is what
/// acts on it. The stream ends when `events` does, or when the client goes and
/// the response is dropped.
#[must_use]
pub fn stream<S>(events: S) -> Response
where
    S: Stream + Send + 'static,
    S::Item: Into<Event>,
{
    unstored(
        Sse::new(events.map(|event| Ok::<_, Infallible>(event.into()))).keep_alive(
            KeepAlive::new()
                .interval(HEARTBEAT)
                .event(Event::default().event(HEARTBEAT_EVENT)),
        ),
    )
}

/// The response with `Cache-Control: no-store` in place of axum's `no-cache`,
/// which lets a cache keep a copy as long as it asks before reusing it.
fn unstored(sse: impl IntoResponse) -> Response {
    let mut response = sse.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
