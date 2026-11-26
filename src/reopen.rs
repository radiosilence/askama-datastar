//! The attributes that keep a page's live stream open.
//!
//! Datastar retries a request whose connection broke, but a stream that ended,
//! as one does when its server stops, stays ended, and a socket that stopped
//! carrying bytes without an error (what a suspended laptop or a changed
//! network hands back) looks live to it for good. A page holding a live stream
//! reopens it itself, and these attributes are how, with no script of the
//! page's own: Datastar's expressions keep the little state they need in one
//! property of their element, `el.__reopen`.
//!
//! They go on an element of their own, `<div hidden{{ live }}></div>`, since
//! they count every request their element makes: a POST from the same element
//! would hold the stream open while in flight, and a refused one would stop it.
//!
//! - `data-on:datastar-fetch` follows the element's requests: how many are open
//!   (`started` and `finished`), when the stream last said anything (any
//!   `datastar-*` event, the heartbeat [`stream`](crate::stream) sends
//!   included, or a request starting), and whether the server refused it. A
//!   401 or 403, such as a session a deploy ended, is a refusal, which asking
//!   again does not get past; every other status is taken for temporary. The
//!   next request to start lifts it.
//! - `data-init` opens the stream after a moment, once every attribute on the
//!   page has been applied, so the first request carries the page's signals
//!   whichever comes first in the document.
//! - `data-on-interval`, every 5 seconds while the page is shown and not
//!   refused, reopens a stream that ended, or one that has said nothing for 40
//!   seconds, against a heartbeat every 15. A server answering with an error,
//!   as a proxy does while the server restarts, is asked again every 5
//!   seconds; one that cannot be reached at all is left to Datastar's own
//!   retries, and asked afresh after 40 quiet seconds.
//! - `data-on:visibilitychange` reopens a stream that ended while the page was
//!   hidden, as soon as it is shown, refused or not, since the person looking
//!   may have signed in elsewhere meanwhile. A stream still open needs nothing:
//!   Datastar closes a GET when the page is hidden and opens it again when the
//!   page is shown. It restarts the quiet clock either way, so the interval
//!   does not take the pause for a lost stream.
//!
//! Reopening uses `@get` with its default `requestCancellation`, which aborts
//! the request already open to the same URL, so there is never more than one
//! stream.

use crate::Html;

/// What follows `data-reopen-url`, the same for every stream. Nothing in it
/// comes from outside: the URL is read from the element with
/// `el.dataset.reopenUrl`, so whatever it holds stays text.
///
/// Each expression makes the element's state the first time one runs. Datastar
/// applies an element's attributes in the order they are written, so
/// `data-on:datastar-fetch` comes first and hears every request the element
/// starts.
const ATTRIBUTES: &str = concat!(
    r#" data-on:datastar-fetch="evt.detail.el === el && (r => "#,
    r#"evt.detail.type === 'started' ? (r.open++, r.seen = Date.now(), r.refused = false)"#,
    r#" : evt.detail.type === 'finished' ? r.open = Math.max(0, r.open - 1)"#,
    r#" : evt.detail.type === 'error' ? r.refused = ['401', '403'].includes(evt.detail.argsRaw.status)"#,
    r#" : evt.detail.type.startsWith('datastar-') && (r.seen = Date.now()))"#,
    r#"(el.__reopen ??= {open: 0, seen: Date.now(), refused: false})""#,
    r#" data-init__delay.10ms="@get(el.dataset.reopenUrl)""#,
    r#" data-on-interval__duration.5s="(r => !r.refused && !document.hidden"#,
    r#" && (!r.open || Date.now() - r.seen > 40000) && @get(el.dataset.reopenUrl))"#,
    r#"(el.__reopen ??= {open: 0, seen: Date.now(), refused: false})""#,
    r#" data-on:visibilitychange__document="document.hidden || (r => (r.seen = Date.now(),"#,
    r#" r.open || @get(el.dataset.reopenUrl)))"#,
    r#"(el.__reopen ??= {open: 0, seen: Date.now(), refused: false})""#,
);

/// How long the stream may say nothing before the page takes it for lost, as
/// [`ATTRIBUTES`] writes it: long enough for two heartbeats to go missing.
#[cfg(test)]
pub(crate) const LOST: std::time::Duration = std::time::Duration::from_secs(40);

/// Attributes that open the live stream at `url` and keep it open, for an
/// element of their own that stays on the page as long as the stream should:
///
/// ```
/// use askama::Template;
/// use askama_datastar::{Html, reopen};
///
/// #[derive(Template)]
/// #[template(ext = "html", source = "<div hidden{{ live }}></div>")]
/// struct Live {
///     live: Html,
/// }
///
/// let live = Live { live: reopen("/stream?tab=queue") }.render()?;
/// assert!(live.starts_with(r#"<div hidden data-reopen-url="/stream?tab=queue" "#));
/// # Ok::<(), askama::Error>(())
/// ```
///
/// The text starts with a space, so it follows the tag's name or its other
/// attributes directly. The element should make no other requests, since the
/// attributes take every request it makes for the stream. `url` is escaped as
/// an attribute value and read back with `el.dataset.reopenUrl`, never written
/// into an expression, so no URL can inject script.
///
/// # Panics
///
/// If `url` is not a path on the page's own origin: it must start with `/`,
/// and not with `//` or `/\`, which a browser reads as another host. A stream
/// is the server's own route, so a URL from elsewhere is a mistake.
#[must_use]
pub fn reopen(url: &str) -> Html {
    assert!(
        url.starts_with('/') && !url.starts_with("//") && !url.starts_with("/\\"),
        "{url:?} is not a path on this origin: a live stream is reopened from the page's own server"
    );
    let mut attributes = String::with_capacity(url.len() + 24 + ATTRIBUTES.len());
    attributes.push_str(r#" data-reopen-url=""#);
    for c in url.chars() {
        match c {
            '&' => attributes.push_str("&amp;"),
            '"' => attributes.push_str("&quot;"),
            '\'' => attributes.push_str("&#39;"),
            '<' => attributes.push_str("&lt;"),
            '>' => attributes.push_str("&gt;"),
            c => attributes.push(c),
        }
    }
    attributes.push('"');
    attributes.push_str(ATTRIBUTES);
    Html::trusted(attributes)
}

#[cfg(test)]
mod tests {
    use super::LOST;
    use crate::sse::HEARTBEAT;

    #[test]
    fn a_stream_is_lost_only_once_two_heartbeats_have_gone_missing() {
        assert!(
            LOST > HEARTBEAT * 2,
            "one late heartbeat would reopen a live stream"
        );
        assert!(
            super::ATTRIBUTES.contains(&format!("> {}", LOST.as_millis())),
            "the attributes' threshold is not LOST"
        );
    }
}
