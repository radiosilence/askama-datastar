//! Serving an embedded file under the URL that names its content.

use askama_datastar::Asset;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::routing::get;
use tower::ServiceExt as _;

static FONT: Asset = Asset::new(&[0, 1, 2, 3, 0xff], "font/woff2");

async fn fetch(uri: &str) -> Response<Body> {
    Router::new()
        .route("/assets/font.woff2", get(FONT.serve()))
        .oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
        .await
        .expect("the router answers")
}

#[test]
fn the_url_names_the_path_and_a_hash_of_the_bytes() {
    // FNV-1a of the five bytes.
    assert_eq!(
        FONT.url("/assets/font.woff2"),
        format!("/assets/font.woff2?v={:016x}", 0x3379_bcd0_c530_506a_u64)
    );
    let other = Asset::new(&[0, 1, 2, 3, 0xfe], "font/woff2");
    assert_ne!(
        other.url("/a"),
        FONT.url("/a"),
        "a changed byte changes the address"
    );
}

#[tokio::test]
async fn an_asset_asked_for_by_its_url_is_kept_for_good() {
    let response = fetch(&FONT.url("/assets/font.woff2")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "font/woff2");
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    assert_eq!(bytes.as_ref(), [0, 1, 2, 3, 0xff], "the bytes, as they are");
}

#[tokio::test]
async fn an_asset_asked_for_any_other_way_is_kept_for_an_hour() {
    for uri in [
        "/assets/font.woff2",
        "/assets/font.woff2?v=0123456789abcdef",
        "/assets/font.woff2?v=",
    ] {
        let response = fetch(uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["cache-control"],
            "public, max-age=3600",
            "{uri} may not be these bytes for long"
        );
    }
}
