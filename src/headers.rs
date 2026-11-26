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
/// Referrer-Policy: same-origin
/// ```
///
/// A header a handler set is kept as it is, so a route with its own policy,
/// such as an image served with `sandbox`, sets it on its response.
///
/// `X-Frame-Options: DENY` repeats `frame-ancestors 'none'` for browsers that
/// read only the older header, so it is sent only while the policy says so: not
/// when the policy allows a frame ancestor, and not on a response that set its
/// own policy, whose `frame-ancestors` it might contradict.
///
/// `Referrer-Policy` is `same-origin` rather than `no-referrer`: a browser sends
/// `Origin: null` on a form's POST under `no-referrer`, which defeats the
/// `Origin` check a plain form such as a sign-in relies on (see
/// [`require_datastar_on_post`](crate::require_datastar_on_post)). Under
/// `same-origin`, requests to this origin carry their origin and referrer, and
/// requests anywhere else carry neither.
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
    deny_frames: bool,
}

impl SecurityHeaders {
    /// The headers, with `csp` as the policy.
    #[must_use]
    pub fn new(csp: &Csp) -> Self {
        Self {
            csp: csp.header_value(),
            deny_frames: csp.denies_frames(),
        }
    }
}

/// The headers with [`Csp::new`]'s policy, for pages that do not load
/// Datastar. A server whose pages run no script at all says so with
/// `SecurityHeaders::new(&Csp::new().deny("script-src"))`.
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
            deny_frames: self.deny_frames,
        }
    }
}

/// The service [`SecurityHeaders`] wraps around another.
#[derive(Clone, Debug)]
pub struct SetSecurityHeaders<S> {
    inner: S,
    csp: HeaderValue,
    deny_frames: bool,
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
            deny_frames: self.deny_frames,
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
        deny_frames: bool,
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
        let own_policy = headers.contains_key(header::CONTENT_SECURITY_POLICY);
        if let Some(csp) = this.csp.take().filter(|_| !own_policy) {
            headers.insert(header::CONTENT_SECURITY_POLICY, csp);
            if *this.deny_frames {
                headers
                    .entry(header::X_FRAME_OPTIONS)
                    .or_insert(HeaderValue::from_static("DENY"));
            }
        }
        headers
            .entry(header::X_CONTENT_TYPE_OPTIONS)
            .or_insert(HeaderValue::from_static("nosniff"));
        headers
            .entry(header::REFERRER_POLICY)
            .or_insert(HeaderValue::from_static("same-origin"));
        Poll::Ready(Ok(response))
    }
}
