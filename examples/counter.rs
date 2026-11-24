//! A counter: the page renders the count, and a button posts to a handler that
//! patches the same template into the page. `cargo run --example counter`, then
//! open <http://127.0.0.1:3000>.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use askama::Template;
use askama_datastar::{Html, events, require_datastar_on_post, script};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::Response,
    routing::{get, post},
};

#[derive(Template)]
#[template(ext = "html", source = r#"<p id="count">{{ n }}</p>"#)]
struct Count {
    n: u64,
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<!doctype html>
<script type="module" src="{{ script }}"></script>
{{ count }}
<button data-on:click="@post('/increment')">Add one</button>"#
)]
struct Page {
    script: String,
    count: Html,
}

type Counter = Arc<AtomicU64>;

async fn page(State(counter): State<Counter>) -> Result<Html, StatusCode> {
    let count = render(&Count {
        n: counter.load(Ordering::Relaxed),
    })?;
    render(&Page {
        script: script::url("/datastar.js"),
        count,
    })
}

async fn increment(State(counter): State<Counter>) -> Result<Response, StatusCode> {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    Ok(events([render(&Count { n })?.patch().into()]))
}

fn render(template: &impl Template) -> Result<Html, StatusCode> {
    Html::render(template).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = Router::new()
        .route("/increment", post(increment))
        .layer(middleware::from_fn(require_datastar_on_post))
        .route("/", get(page))
        .route("/datastar.js", get(script::serve))
        .with_state(Counter::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
