//! The security headers every response carries, as a tower layer.
//!
//! A layer rather than middleware written with `from_fn`, so that its types
//! can be named and it costs no allocation per request. It adds each header
//! only where the response lacks it: a route that needs a stricter policy, or
//! a wider one, sets its own and the layer leaves it alone.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use axum::http::{HeaderValue, Response, header};
use pin_project_lite::pin_project;

use crate::Csp;

/// A layer setting, on every response that lacks them:
///
/// ```text
/// Content-Security-Policy: <the policy it was given>
/// X-Content-Type-Options: nosniff
/// X-Frame-Options: DENY
/// Referrer-Policy: no-referrer
/// ```
///
/// `X-Frame-Options` repeats `frame-ancestors 'none'` for browsers that read
/// only the older header. A header a handler set is kept as it is, so a route
/// with its own policy, such as an image served with `sandbox`, sets it on its
/// response.
///
/// ```
/// use askama_datastar::{Csp, SecurityHeaders};
/// use axum::Router;
/// use axum::routing::get;
///
/// let app: Router = Router::new()
///     .route("/", get(|| async { "a page" }))
///     .layer(SecurityHeaders::new(&Csp::new().datastar()));
/// ```
#[derive(Clone, Debug)]
pub struct SecurityHeaders {
    csp: HeaderValue,
}

impl SecurityHeaders {
    /// The headers, with `csp` as the policy.
    #[must_use]
    pub fn new(csp: &Csp) -> Self {
        Self {
            csp: csp.header_value(),
        }
    }
}

/// The headers with [`Csp::new`]'s policy, for a server with no script.
impl Default for SecurityHeaders {
    fn default() -> Self {
        Self::new(&Csp::new())
    }
}

impl<S> tower_layer::Layer<S> for SecurityHeaders {
    type Service = SetSecurityHeaders<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SetSecurityHeaders {
            inner,
            csp: self.csp.clone(),
        }
    }
}

/// The service [`SecurityHeaders`] wraps around another.
#[derive(Clone, Debug)]
pub struct SetSecurityHeaders<S> {
    inner: S,
    csp: HeaderValue,
}

impl<S, Request, Body> tower_service::Service<Request> for SetSecurityHeaders<S>
where
    S: tower_service::Service<Request, Response = Response<Body>>,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = SecurityHeadersFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        SecurityHeadersFuture {
            inner: self.inner.call(request),
            csp: Some(self.csp.clone()),
        }
    }
}

pin_project! {
    /// The response of a [`SetSecurityHeaders`], with the headers added.
    #[derive(Debug)]
    pub struct SecurityHeadersFuture<F> {
        #[pin]
        inner: F,
        csp: Option<HeaderValue>,
    }
}

impl<F, Body, E> Future for SecurityHeadersFuture<F>
where
    F: Future<Output = Result<Response<Body>, E>>,
{
    type Output = Result<Response<Body>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut response = ready!(this.inner.poll(cx))?;
        let headers = response.headers_mut();
        if let Some(csp) = this.csp.take() {
            headers
                .entry(header::CONTENT_SECURITY_POLICY)
                .or_insert(csp);
        }
        headers
            .entry(header::X_CONTENT_TYPE_OPTIONS)
            .or_insert(HeaderValue::from_static("nosniff"));
        headers
            .entry(header::X_FRAME_OPTIONS)
            .or_insert(HeaderValue::from_static("DENY"));
        headers
            .entry(header::REFERRER_POLICY)
            .or_insert(HeaderValue::from_static("no-referrer"));
        Poll::Ready(Ok(response))
    }
}
