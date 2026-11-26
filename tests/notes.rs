//! The notes example, end to end: its pages whole and in place, its actions,
//! its live stream, and the escaping guard over every answer.

#[allow(dead_code)]
#[path = "../examples/notes.rs"]
mod notes;

use std::time::Duration;

use askama_datastar::escaping::{Guard, escaped, get};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

const HOSTILE: &str = r#"<em>hostile "q" & 'x'</em>"#;
const WAIT: Duration = Duration::from_millis(500);

fn board() -> (notes::Notes, Router) {
    let board = notes::Notes::default();
    (board.clone(), notes::routes(board))
}

fn encoded(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// The stream's address, with the signals of the page at `url`.
fn live(url: &str) -> String {
    let signals = serde_json::json!({ "url": url, "text": "" });
    format!("/live?datastar={}", encoded(&signals.to_string()))
}

#[tokio::test]
async fn markup_from_people_is_escaped_once_everywhere() {
    let (board, router) = board();
    let id = board.add(HOSTILE);
    let guard = Guard::new(&[HOSTILE]).markup(include_str!("../examples/notes.rs"));
    let search = format!("/?q={}", encoded(HOSTILE));
    let note = format!("/notes/{id}");
    let mut found = Vec::new();
    for (url, shows) in [
        ("/", true),
        (&search[..], true),
        (&note[..], true),
        ("/about", false),
    ] {
        for in_place in [false, true] {
            let (status, answer) = get(&router, url, in_place, WAIT).await;
            assert!(status.is_success(), "{url}: {status}");
            found.extend(
                guard
                    .problems(&answer)
                    .iter()
                    .map(|p| format!("{url}: {p}")),
            );
            if shows && !answer.contains(&escaped(HOSTILE)) {
                found.push(format!("{url}: the note is missing"));
            }
        }
        let (_, answer) = get(&router, &live(url), true, WAIT).await;
        found.extend(
            guard
                .problems(&answer)
                .iter()
                .map(|p| format!("{url} live: {p}")),
        );
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[tokio::test]
async fn a_page_loads_whole_and_in_place() {
    let (board, router) = board();
    board.add("moss");
    let (status, page) = get(&router, "/?q=mo", false, WAIT).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("<title id=\"title\">notes · notes</title>"),
        "{page}"
    );
    assert!(
        page.contains("&quot;url&quot;:&quot;/?q=mo&quot;"),
        "{page}"
    );
    assert!(page.contains(r#"data-reopen-url="/live""#), "{page}");
    assert!(page.contains("<li><a href=\"/notes/0\""), "{page}");

    let (_, events) = get(&router, "/notes/0?datastar=%7B%7D", true, WAIT).await;
    assert!(events.contains("data: selector #main"), "{events}");
    assert!(
        events.contains("<title id=\"title\">moss · notes</title>"),
        "{events}"
    );
    assert!(
        events.contains(r#"data: signals {"url":"/notes/0"}"#),
        "{events}"
    );
}

#[tokio::test]
async fn a_missing_note_is_404_and_the_latest_redirects() {
    let (board, router) = board();
    assert_eq!(
        get(&router, "/notes/7", false, WAIT).await.0,
        StatusCode::NOT_FOUND
    );
    board.add("moss");
    let response = router
        .clone()
        .oneshot(Request::get("/latest").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/notes/0");
    let (_, events) = get(&router, "/latest", true, WAIT).await;
    assert!(events.contains(r#"data-go="/notes/0""#), "{events}");
}

async fn post(router: &Router, uri: &str, json: &str) -> String {
    let request = Request::post(uri)
        .header("datastar-request", "true")
        .header("content-type", "application/json")
        .body(Body::from(json.to_owned()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn actions_answer_with_as_little_as_they_can() {
    let (board, router) = board();
    let refused = post(&router, "/notes", r#"{"text":"  "}"#).await;
    assert!(refused.contains("Write something first."), "{refused}");

    let added = post(&router, "/notes", r#"{"text":"moss","url":"/"}"#).await;
    assert!(
        added.contains("data: selector #error"),
        "the error cleared: {added}"
    );
    assert!(board.add("lichen") == 1);

    let deleted = post(&router, "/notes/0/delete", "{}").await;
    assert!(
        deleted.contains(r#"data: signals {"url":"/"}"#),
        "{deleted}"
    );
    assert!(
        deleted.contains("lichen") && !deleted.contains(">moss<"),
        "{deleted}"
    );
}

#[tokio::test]
async fn the_stream_draws_a_note_added_elsewhere() {
    let (board, router) = board();
    let reader = {
        let router = router.clone();
        tokio::spawn(async move {
            get(&router, &live("/"), true, Duration::from_secs(1))
                .await
                .1
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    board.add("moss");
    let streamed = reader.await.unwrap();
    assert!(streamed.contains("1 notes"), "{streamed}");
    assert!(streamed.contains(">moss</a>"), "{streamed}");
}
