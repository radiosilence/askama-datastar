//! The Content Security Policy and the headers every response carries.

use askama_datastar::{Csp, SecurityHeaders};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, header};
use axum::routing::get;
use tower::ServiceExt as _;

/// The policy STANDARD §8 gives every page.
const STANDARD: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; font-src 'self'; connect-src 'self'; form-action 'self'; \
     frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

#[test]
fn the_policy_starts_as_the_standard_gives_it() {
    assert_eq!(Csp::new().to_string(), STANDARD);
    assert_eq!(Csp::default(), Csp::new());
}

#[test]
fn a_page_loading_datastar_may_eval_its_own_script_and_nothing_inline() {
    let csp = Csp::new().datastar().to_string();
    assert!(
        csp.contains("; script-src 'self' 'unsafe-eval';"),
        "Datastar compiles its expressions with Function: {csp}"
    );
    assert!(!csp.contains("unsafe-inline"), "an inline hatch: {csp}");
}

#[test]
fn an_import_map_is_allowed_by_the_hash_of_its_text() {
    let map = r#"{"imports":{"app/player":"/assets/player.js?v=0123456789abcdef"}}"#;
    let csp = Csp::new().datastar().import_map(map).to_string();
    // `openssl dgst -sha256 -binary | base64` over the same text.
    assert!(
        csp.contains(
            "script-src 'self' 'unsafe-eval' 'sha256-7VrnJpZg0ZEp+SRswcOIyb4vBrzBA186XK2Smfxgsb0=';"
        ),
        "{csp}"
    );
}

#[test]
fn a_page_with_no_script_runs_none() {
    let csp = Csp::new().deny("script-src").to_string();
    assert!(csp.contains("; script-src 'none';"), "{csp}");
}

#[test]
fn a_source_is_added_once_and_replaces_none() {
    let csp = Csp::new()
        .allow("img-src", "blob:")
        .allow("img-src", "blob:")
        .allow("frame-ancestors", "https://example.com")
        .allow("media-src", "'self'")
        .to_string();
    assert!(csp.contains("img-src 'self' data: blob:;"), "{csp}");
    assert!(
        csp.contains("frame-ancestors https://example.com;"),
        "'none' and a source together mean the source: {csp}"
    );
    assert!(
        csp.ends_with("; media-src 'self'"),
        "a new directive goes last: {csp}"
    );
}

#[test]
fn a_denied_directive_is_none_alone() {
    let csp = Csp::new()
        .deny("default-src")
        .deny("worker-src")
        .to_string();
    assert!(csp.starts_with("default-src 'none'; script-src"), "{csp}");
    assert!(csp.ends_with("; worker-src 'none'"), "{csp}");
}

#[test]
#[should_panic(expected = "is not a CSP source")]
fn a_source_cannot_end_its_directive() {
    let _ = Csp::new().allow("img-src", "https://a.example; script-src *");
}

#[test]
#[should_panic(expected = "is not a CSP directive name")]
fn a_directive_name_is_one_word() {
    let _ = Csp::new().allow("img-src https:", "blob:");
}

async fn fetch(app: Router, uri: &str) -> Response<Body> {
    app.oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
        .await
        .expect("the router answers")
}

#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let app = Router::new()
        .route("/", get(|| async { "a page" }))
        .layer(SecurityHeaders::new(&Csp::new().datastar()));
    for uri in ["/", "/nowhere"] {
        let response = fetch(app.clone(), uri).await;
        let headers = response.headers();
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            Csp::new().datastar().to_string(),
            "{uri}"
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{uri}");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{uri}");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{uri}");
    }
}

#[tokio::test]
async fn a_header_the_handler_set_is_kept() {
    let sandboxed = || async {
        (
            [
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; sandbox",
                ),
                (header::REFERRER_POLICY, "same-origin"),
            ],
            "an image",
        )
    };
    let app = Router::new()
        .route("/file", get(sandboxed))
        .layer(SecurityHeaders::default());
    let response = fetch(app, "/file").await;
    let headers = response.headers();
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; sandbox",
        "the route's own policy was replaced"
    );
    assert_eq!(headers[header::REFERRER_POLICY], "same-origin");
    assert_eq!(
        headers
            .get_all(header::CONTENT_SECURITY_POLICY)
            .iter()
            .count(),
        1,
        "a second policy would narrow the route's"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}
