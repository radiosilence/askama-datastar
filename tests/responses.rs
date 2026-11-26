//! The responses events go out in: their headers, the heartbeat that holds a
//! quiet stream open, and when each ends.

use std::time::Duration;

use askama_datastar::{Html, events, stream};
use axum::body::Body;
use futures_util::StreamExt as _;
use tokio::time::Instant;

#[test]
fn an_answer_is_an_event_stream_no_cache_may_keep() {
    let response = events([Html::trusted("<p id=\"a\"></p>").patch().into()]);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn a_live_stream_is_an_event_stream_no_cache_may_keep() {
    let response = stream(futures_util::stream::empty::<askama_datastar::PatchElements>());
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-store");
}

/// The next chunk of `body`, as text.
async fn next(
    body: &mut (impl futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
) -> Option<String> {
    let chunk = body.next().await?.expect("the body does not fail");
    Some(String::from_utf8(chunk.to_vec()).expect("events are UTF-8"))
}

#[tokio::test(start_paused = true)]
async fn a_quiet_stream_sends_a_heartbeat_every_fifteen_seconds() {
    let patches = futures_util::stream::once(async { Html::trusted("<p id=\"a\"></p>").patch() })
        .chain(futures_util::stream::pending());
    let mut body = Body::new(stream(patches).into_body()).into_data_stream();
    let started = Instant::now();

    assert_eq!(
        next(&mut body).await.as_deref(),
        Some("event: datastar-patch-elements\ndata: elements <p id=\"a\"></p>\n\n"),
    );
    // Named `datastar-…`, or Datastar would not tell the page it came.
    assert_eq!(
        next(&mut body).await.as_deref(),
        Some("event: datastar-heartbeat\n\n")
    );
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(15),
        "the heartbeat came after the quiet it bridges"
    );
    assert_eq!(
        next(&mut body).await.as_deref(),
        Some("event: datastar-heartbeat\n\n")
    );
    assert_eq!(started.elapsed(), Duration::from_secs(30));
}

#[tokio::test]
async fn a_stream_ends_when_its_events_do() {
    let patches = futures_util::stream::iter([
        Html::trusted("<p id=\"a\">1</p>").patch(),
        Html::trusted("<p id=\"a\">2</p>").patch(),
    ]);
    let bytes = axum::body::to_bytes(stream(patches).into_body(), usize::MAX)
        .await
        .expect("a finished stream is a finished body");
    assert_eq!(
        String::from_utf8(bytes.to_vec()).expect("events are UTF-8"),
        concat!(
            "event: datastar-patch-elements\ndata: elements <p id=\"a\">1</p>\n\n",
            "event: datastar-patch-elements\ndata: elements <p id=\"a\">2</p>\n\n",
        ),
    );
}

#[tokio::test]
async fn an_answer_with_no_events_is_an_empty_stream() {
    let bytes = axum::body::to_bytes(events(Vec::new()).into_body(), usize::MAX)
        .await
        .expect("an empty answer is a finished body");
    assert!(bytes.is_empty(), "an empty answer sent {bytes:?}");
}
