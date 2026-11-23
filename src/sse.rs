//! Answers in server-sent events: a few events at once, or a live stream.
//!
//! Both take anything that becomes an axum SSE [`Event`]: the SDK's patches
//! and signals, or an `Event` written by hand. Neither is stored by a cache,
//! since what they carry is the page as one person sees it.

use std::convert::Infallible;

use axum::http::{HeaderValue, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{Stream, StreamExt as _};

/// `events` as one answer, which ends once the last is sent.
///
/// ```
/// use askama_datastar::{Html, PatchSignals, events};
///
/// let response = events([
///     Html::trusted(r#"<p id="flash">Saved.</p>"#).patch().into(),
///     PatchSignals::new(r#"{"name":""}"#).into(),
/// ]);
/// assert_eq!(response.headers()["content-type"], "text/event-stream");
/// ```
pub fn events<I>(events: I) -> Response
where
    I: IntoIterator,
    I::Item: Into<Event>,
{
    let events: Vec<Event> = events.into_iter().map(Into::into).collect();
    unstored(Sse::new(futures_util::stream::iter(
        events.into_iter().map(Ok::<_, Infallible>),
    )))
}

/// `events` as they come, for as long as the stream runs.
///
/// A comment goes down the connection after 15 seconds without an event, so a
/// proxy that closes a quiet connection leaves it open. The stream ends when
/// `events` does, or when the client goes and the response is dropped.
pub fn stream<S>(events: S) -> Response
where
    S: Stream + Send + 'static,
    S::Item: Into<Event>,
{
    unstored(
        Sse::new(events.map(|event| Ok::<_, Infallible>(event.into())))
            .keep_alive(KeepAlive::default()),
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
