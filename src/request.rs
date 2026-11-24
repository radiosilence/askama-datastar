//! What a request says about itself: whether Datastar sent it, and the signals
//! it carries.
//!
//! Datastar marks every request it makes with a `Datastar-Request` header. A
//! cross-site form cannot set a header, and a cross-site script cannot set one
//! without a CORS preflight the server never answers, so the header is what
//! tells Datastar's requests from page loads and from forgeries.

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

/// The header Datastar sets on every request it makes.
pub(crate) const HEADER: &str = "datastar-request";

/// Whether Datastar sent the request.
///
/// A page load is not, and wants a whole page; a request the page makes
/// wants a patch, and a status rather than a redirect, since Datastar follows
/// a redirect like any fetch and an event stream cannot be redirected at all.
#[must_use]
pub fn is_datastar(headers: &HeaderMap) -> bool {
    headers.contains_key(HEADER)
}

/// Middleware refusing, with 403, a POST that Datastar did not send.
///
/// Layered over the routes Datastar posts to, it refuses a POST without the
/// header: one that came from a form or a script on another site. That stops
/// cross-site request forgery whatever the cookie's `SameSite` allows: a
/// sibling subdomain counts as the same site. A plain form, such as a sign-in,
/// posts to a route outside the layer, whose handler checks `Sec-Fetch-Site`
/// or `Origin` itself.
///
/// POST is the one method that needs it. A form sends only GET and POST, and a
/// script on another site cannot send PUT, PATCH or DELETE at all without a
/// CORS preflight the server never answers.
///
/// ```
/// use axum::Router;
/// use axum::middleware::from_fn;
/// use axum::routing::post;
///
/// let app: Router = Router::new()
///     .route("/save", post(|| async { "saved" }))
///     .layer(from_fn(askama_datastar::require_datastar_on_post));
/// ```
pub async fn require_datastar_on_post(request: Request, next: Next) -> Response {
    if request.method() == Method::POST && !is_datastar(request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// The signals a Datastar request carries, as a `T`.
///
/// Datastar sends them as JSON, in the `datastar` query parameter of a GET or
/// DELETE and as the body of anything else. Signals that are missing, or that
/// do not parse as a `T`, read as `T::default()`: they describe what a page
/// shows, and a view whose state does not parse (an address someone edited, a
/// page left open across a release that renamed a signal) is drawn as its
/// default rather than refused.
///
/// A request that changes something should refuse what it cannot read
/// instead, so that a body nobody meant to send changes nothing: take
/// [`datastar::axum::ReadSignals`] or axum's `Json` there, into a struct that
/// accepts fields it does not name, since Datastar sends every signal on the
/// page.
///
/// It reads the body, so it is the last of a handler's extractors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signals<T>(pub T);

impl<S, T> FromRequest<S> for Signals<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Default,
{
    /// A body over the router's limit, or one that broke off: a failure of the
    /// request rather than of its signals, so it is answered as axum answers
    /// it.
    type Rejection = BytesRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let signals = match *request.method() {
            Method::GET | Method::HEAD | Method::DELETE => {
                request.uri().query().and_then(from_query)
            }
            _ => serde_json::from_slice(&Bytes::from_request(request, state).await?).ok(),
        };
        Ok(Self(signals.unwrap_or_default()))
    }
}

/// The `datastar` parameter of `query`, parsed; the first, if it repeats.
fn from_query<T: DeserializeOwned>(query: &str) -> Option<T> {
    form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == "datastar")
        .and_then(|(_, json)| serde_json::from_str(&json).ok())
}
