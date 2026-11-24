//! Telling Datastar's requests apart, refusing a POST it did not send, and
//! reading the signals it carries.

use askama_datastar::{Signals, is_datastar, require_datastar_on_post};
use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::from_fn;
use axum::routing::{get, post};
use serde::Deserialize;
use tower::ServiceExt as _;

#[test]
fn a_request_with_datastars_header_is_datastars() {
    let mut headers = HeaderMap::new();
    assert!(!is_datastar(&headers), "a page load is not Datastar's");
    headers.insert("datastar-request", "true".parse().expect("a header value"));
    assert!(is_datastar(&headers));
}

fn guarded() -> Router {
    Router::new()
        .route(
            "/save",
            post(|| async { "saved" })
                .get(|| async { "page" })
                .put(|| async { "put" }),
        )
        .layer(from_fn(require_datastar_on_post))
}

async fn status(app: Router, request: Request<Body>) -> StatusCode {
    app.oneshot(request)
        .await
        .expect("the router answers")
        .status()
}

#[tokio::test]
async fn a_post_without_datastars_header_is_refused() {
    let forged = Request::post("/save")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(status(guarded(), forged).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_post_from_datastar_goes_through() {
    let request = Request::post("/save")
        .header("datastar-request", "true")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(status(guarded(), request).await, StatusCode::OK);
}

#[tokio::test]
async fn other_methods_are_left_to_the_browsers_preflight() {
    // A form cannot send a PUT, and a script on another site cannot send one
    // without a CORS preflight, so only a POST needs the header.
    let request = Request::put("/save")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(status(guarded(), request).await, StatusCode::OK);
}

#[tokio::test]
async fn a_page_load_is_not_asked_for_the_header() {
    let request = Request::get("/save")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(status(guarded(), request).await, StatusCode::OK);
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
struct Search {
    q: String,
    page: u32,
}

fn app() -> Router {
    async fn read(Signals(search): Signals<Search>) -> String {
        format!("{}:{}", search.q, search.page)
    }
    Router::new()
        .route("/search", get(read).post(read).delete(read).put(read))
        .layer(DefaultBodyLimit::max(64))
}

async fn read(request: Request<Body>) -> (StatusCode, String) {
    let response = app().oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    (status, String::from_utf8(bytes.to_vec()).expect("UTF-8"))
}

fn get_with(query: &str) -> Request<Body> {
    Request::get(format!("/search?{query}"))
        .header("datastar-request", "true")
        .body(Body::empty())
        .expect("a request")
}

fn with_body(method: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/search")
        .header("datastar-request", "true")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("a request")
}

#[tokio::test]
async fn a_get_carries_its_signals_in_the_datastar_parameter() {
    // `{"q":"moss","page":2}`, as Datastar encodes it.
    let request = get_with("datastar=%7B%22q%22%3A%22moss%22%2C%22page%22%3A2%7D");
    assert_eq!(read(request).await, (StatusCode::OK, "moss:2".into()));
}

#[tokio::test]
async fn other_parameters_beside_the_signals_are_left_alone() {
    let request = get_with("tab=3&datastar=%7B%22q%22%3A%22fern%22%7D");
    assert_eq!(read(request).await, (StatusCode::OK, "fern:0".into()));
}

#[tokio::test]
async fn a_delete_carries_its_signals_in_the_query_as_a_get_does() {
    let request = Request::delete("/search?datastar=%7B%22page%22%3A4%7D")
        .header("datastar-request", "true")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(read(request).await, (StatusCode::OK, ":4".into()));
}

#[tokio::test]
async fn a_post_carries_its_signals_as_the_body() {
    let request = with_body("POST", r#"{"q":"lichen","page":1}"#);
    assert_eq!(read(request).await, (StatusCode::OK, "lichen:1".into()));
}

#[tokio::test]
async fn any_method_with_a_body_carries_its_signals_there() {
    let request = with_body("PUT", r#"{"q":"liverwort"}"#);
    assert_eq!(read(request).await, (StatusCode::OK, "liverwort:0".into()));
}

#[tokio::test]
async fn signals_that_do_not_parse_read_as_the_default_view() {
    for request in [
        get_with("datastar=not-json"),
        get_with("datastar=%7B%22page%22%3A%22two%22%7D"),
        get_with(""),
        with_body("POST", "not json"),
        with_body("POST", ""),
    ] {
        assert_eq!(read(request).await, (StatusCode::OK, ":0".into()));
    }
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_rather_than_read_as_the_default() {
    let request = with_body(
        "POST",
        r#"{"q":"a body longer than the sixty-four bytes this router takes"}"#,
    );
    assert_eq!(read(request).await.0, StatusCode::PAYLOAD_TOO_LARGE);
}
