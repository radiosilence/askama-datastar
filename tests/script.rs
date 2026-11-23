//! Serving the vendored script under the URL that names its copy.

use askama_datastar::script;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::routing::get;
use tower::ServiceExt as _;

async fn fetch(uri: &str) -> Response<Body> {
    Router::new()
        .route("/assets/datastar.js", get(script::serve))
        .oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
        .await
        .expect("the router answers")
}

#[test]
fn the_url_names_the_path_and_a_hash_of_the_script() {
    let url = script::url("/assets/datastar.js");
    let hash = url
        .strip_prefix("/assets/datastar.js?v=")
        .expect("the path, then the query");
    assert_eq!(hash.len(), 16, "a 64-bit hash in hex, not {hash:?}");
    assert!(
        hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "{hash:?} is not hex"
    );
    assert_eq!(
        script::url("./ui/assets/datastar.js"),
        format!("./ui/assets/datastar.js?v={hash}")
    );
}

#[tokio::test]
async fn the_script_asked_for_by_its_url_is_kept_for_good() {
    let response = fetch(&script::url("/assets/datastar.js")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    assert_eq!(
        bytes,
        script::SOURCE.as_bytes(),
        "the vendored script, as it is"
    );
}

#[tokio::test]
async fn the_script_asked_for_any_other_way_is_kept_for_an_hour() {
    for uri in [
        "/assets/datastar.js",
        "/assets/datastar.js?v=0123456789abcdef",
    ] {
        let response = fetch(uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["cache-control"],
            "public, max-age=3600",
            "{uri} may not be this copy for long"
        );
    }
}

#[test]
fn the_script_is_datastars_module() {
    assert!(script::SOURCE.starts_with(&format!("// Datastar v{}\n", script::VERSION)));
    assert!(
        script::SOURCE.contains("datastar-fetch"),
        "the vendored file is not Datastar's bundle"
    );
}
