//! A live stream's loop: everything drawn once, a redraw per change at most
//! every throttle, unchanged regions left out, and an end after its lifetime.

#![allow(clippy::unused_async_trait_impl)]

use std::sync::Arc;
use std::time::Duration;

use askama_datastar::Html;
use askama_datastar::live::{self, Frame, Last, Live};
use axum::Router;
use axum::body::{Body, BodyDataStream};
use axum::http::Request;
use axum::response::sse::Event;
use axum::routing::get;
use futures_util::StreamExt as _;
use tokio::sync::{Notify, watch};
use tower::ServiceExt as _;

#[derive(Clone)]
struct App {
    count: Arc<watch::Sender<u64>>,
    /// Woken for the stream's own change.
    reply: Arc<Notify>,
}

/// The count changing, and a header showing it on every page.
struct Counter {
    count: watch::Receiver<u64>,
    header: Last,
}

impl Frame for Counter {
    type State = App;
    type Place = &'static str;
    type Changed = bool;
    const LIFETIME: Duration = Duration::from_secs(5);

    async fn open(app: &App, _: &&'static str) -> Self {
        Self {
            count: app.count.subscribe(),
            header: Last::new(),
        }
    }

    fn everything() -> bool {
        true
    }

    async fn changed(&mut self) -> Option<bool> {
        self.count.changed().await.ok().map(|()| true)
    }

    async fn draw(&mut self, _: &App, changed: &bool) -> Vec<Event> {
        if !changed {
            return Vec::new();
        }
        let n = *self.count.borrow_and_update();
        // Only whether it is even shows, so most changes draw the same header.
        let header = Html::trusted(format!("<p id=\"header\">{}</p>", n % 2));
        self.header.send(&header).into_iter().collect()
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Shown {
    label: String,
}

struct Page {
    label: String,
    reply: Arc<Notify>,
    replies: u32,
}

impl Live for Page {
    type Frame = Counter;
    const PLACE: &'static str = "page";
    type Signals = Shown;

    async fn open(app: &App, shown: Shown) -> Self {
        Self {
            label: shown.label,
            reply: app.reply.clone(),
            replies: 0,
        }
    }

    async fn draw(&mut self, app: &App, changed: &bool) -> Vec<Event> {
        let n = *app.count.borrow();
        let what = if *changed { "count" } else { "reply" };
        vec![
            Html::trusted(format!(
                "<p id=\"page\">{} {what} {n} {}</p>",
                self.label, self.replies
            ))
            .patch()
            .into(),
        ]
    }

    async fn wake(&mut self) {
        self.reply.notified().await;
        self.replies += 1;
    }
}

async fn open() -> (App, BodyDataStream) {
    let app = App {
        count: Arc::new(watch::channel(0).0),
        reply: Arc::new(Notify::new()),
    };
    let router = Router::new()
        .route("/stream", get(live::serve::<Page>))
        .with_state(app.clone());
    let response = router
        .oneshot(
            Request::get("/stream?datastar=%7B%22label%22%3A%22jobs%22%7D")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    (app, response.into_body().into_data_stream())
}

async fn next(body: &mut BodyDataStream) -> String {
    let chunk = body.next().await.expect("an event").unwrap();
    String::from_utf8(chunk.to_vec()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_stream_draws_everything_then_each_change() {
    let (app, mut body) = open().await;
    assert!(next(&mut body).await.contains("<p id=\"header\">0</p>"));
    assert!(
        next(&mut body)
            .await
            .contains("<p id=\"page\">jobs count 0 0</p>")
    );

    app.count.send_replace(2);
    let page = next(&mut body).await;
    assert!(
        page.contains("<p id=\"page\">jobs count 2 0</p>"),
        "the header is what was last sent, so only the page is: {page}"
    );

    app.count.send_replace(3);
    assert!(next(&mut body).await.contains("<p id=\"header\">1</p>"));
    assert!(next(&mut body).await.contains("jobs count 3 0"));

    app.reply.notify_one();
    assert!(next(&mut body).await.contains("jobs reply 3 1"));
}

#[tokio::test(start_paused = true)]
async fn changes_inside_the_throttle_are_one_redraw() {
    let (app, mut body) = open().await;
    next(&mut body).await;
    next(&mut body).await;
    let start = tokio::time::Instant::now();
    app.count.send_replace(2);
    assert!(next(&mut body).await.contains("count 2"));
    app.count.send_replace(4);
    app.count.send_replace(6);
    assert!(
        next(&mut body).await.contains("count 6"),
        "4 is never drawn"
    );
    assert!(start.elapsed() >= Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn a_stream_ends_after_its_lifetime() {
    let (_app, mut body) = open().await;
    next(&mut body).await;
    next(&mut body).await;
    let start = tokio::time::Instant::now();
    let mut rest = String::new();
    while let Some(chunk) = body.next().await {
        rest.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
    }
    assert_eq!(rest, "", "nothing changed, so nothing was sent");
    assert_eq!(start.elapsed().as_secs(), 5);
}
