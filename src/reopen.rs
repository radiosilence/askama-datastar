//! The attributes that keep a page's live stream open.
//!
//! Datastar retries a request whose connection broke, but a stream that ended,
//! as one does when its server stops, stays ended, and a socket that stopped
//! carrying bytes without an error (what a suspended laptop or a changed
//! network hands back) looks live to it for good. A page holding a live stream
//! reopens it itself, and these attributes are how, with no script of the
//! page's own: Datastar's expressions, on the element they are written on,
//! keep the little state they need as properties of that element.
//!
//! - `data-on:datastar-fetch` follows this element's requests: how many are
//!   open (`started` and `finished`), when the stream last said anything (any
//!   `datastar-*` event, the heartbeat [`stream`](crate::stream) sends
//!   included, or a request starting), and whether the server refused it (a
//!   4xx, such as the 401 of a session a deploy ended, which asking again does
//!   not get past; a frame from a later request lifts it).
//! - `data-init` opens the stream, once the attribute before it is listening.
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

/// What follows `data-url`, the same for every stream. Nothing in it comes
/// from outside: the URL is read from the element with `el.dataset.url`, so
/// whatever it holds stays text.
///
/// Datastar applies an element's attributes in the order they are written, and
/// `@get` announces its start before it first waits, so `data-on:datastar-fetch`
/// comes before `data-init`: the other way round, the first stream's start goes
/// unheard and the page takes it for ended.
const ATTRIBUTES: &str = concat!(
    r#" data-on:datastar-fetch="evt.detail.el === el && ("#,
    r#"evt.detail.type === 'started' ? (el.open = (el.open || 0) + 1, el.seen = Date.now())"#,
    r#" : evt.detail.type === 'finished' ? el.open = Math.max(0, (el.open || 0) - 1)"#,
    r#" : evt.detail.type === 'error' ? el.refused = evt.detail.argsRaw.status < 500"#,
    r#" : evt.detail.type.startsWith('datastar-') && (el.refused = false, el.seen = Date.now()))""#,
    r#" data-init="@get(el.dataset.url)""#,
    r#" data-on-interval__duration.5s="!el.refused && !document.hidden"#,
    r#" && (!el.open || Date.now() - el.seen > 40000) && @get(el.dataset.url)""#,
    r#" data-on:visibilitychange__document="document.hidden"#,
    r#" || (el.seen = Date.now(), el.open || @get(el.dataset.url))""#,
);

/// How long the stream may say nothing before the page takes it for lost, as
/// [`ATTRIBUTES`] writes it: long enough for two heartbeats to go missing.
#[cfg(test)]
pub(crate) const LOST: std::time::Duration = std::time::Duration::from_secs(40);

/// Attributes that open the live stream at `url` and keep it open, for the
/// opening tag of an element that stays on the page as long as the stream
/// should, such as `<body>`:
///
/// ```
/// use askama::Template;
/// use askama_datastar::{Html, reopen};
///
/// #[derive(Template)]
/// #[template(ext = "html", source = "<body{{ live }}></body>")]
/// struct Page {
///     live: Html,
/// }
///
/// let page = Page { live: reopen("/stream?tab=queue") }.render()?;
/// assert!(page.starts_with(r#"<body data-url="/stream?tab=queue" data-on:datastar-fetch="#));
/// # Ok::<(), askama::Error>(())
/// ```
///
/// The text starts with a space, so it follows the tag's name or its other
/// attributes directly. `url` is escaped as an attribute value and read back
/// with `el.dataset.url`, never written into an expression, so any URL is safe
/// here, one built from what a person typed included.
///
/// A stream the page opens some other way, such as one that carries a form,
/// is not reopened by these; they own the requests to `url` made from this
/// element.
#[must_use]
pub fn reopen(url: &str) -> Html {
    let mut attributes = String::with_capacity(url.len() + 16 + ATTRIBUTES.len());
    attributes.push_str(r#" data-url=""#);
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
