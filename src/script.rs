//! Datastar's own script, vendored and pinned.
//!
//! `assets/datastar.js` is the `bundles/datastar.js` of the Datastar release
//! [`VERSION`] names, byte for byte: `scripts/datastar.sh` writes it, and CI
//! downloads the release's copy again to check nothing else has. Serving it
//! from the binary keeps a page's scripts on its own origin, as a CSP of
//! `script-src 'self'` requires, and one pinned copy here moves every
//! dependent to a new release together.
//!
//! Datastar compiles its attribute expressions with `Function`, so a page that
//! loads it needs `'unsafe-eval'` in its `script-src`, which
//! [`Csp::datastar`](crate::Csp::datastar) adds.

use axum::extract::RawQuery;
use axum::response::Response;

use crate::Asset;

/// The Datastar release vendored.
pub const VERSION: &str = "1.0.5";

/// The script, as that release publishes it.
pub const SOURCE: &str = include_str!("../assets/datastar.js");

static SCRIPT: Asset = Asset::new(SOURCE.as_bytes(), "text/javascript; charset=utf-8");

/// `path` with the query naming this copy of the script, for a page's
/// `<script type="module" src="…">`: `/assets/datastar.js?v=…`.
///
/// The query changes whenever the script does, so a page from a new build
/// fetches its own script rather than a cached copy of the last one, and
/// [`serve`] can let a browser keep it for good. It is [`Asset::url`]'s.
#[must_use]
pub fn url(path: &str) -> String {
    SCRIPT.url(path)
}

/// The script, as a handler for the route [`url`]'s `path` names.
///
/// Asked for by the URL [`url`] gives, it is cached for a year as immutable.
/// Asked for any other way, by a bare path or a page from an earlier build, it
/// is cached for an hour, since that address will not always be this copy.
#[expect(
    clippy::unused_async,
    reason = "axum takes a handler as an async fn, and this one has nothing to wait for"
)]
pub async fn serve(RawQuery(query): RawQuery) -> Response {
    SCRIPT.response(query.as_deref())
}

#[cfg(test)]
mod tests {
    use super::{SOURCE, VERSION};

    #[test]
    fn the_vendored_script_is_the_release_version_names() {
        let first = SOURCE.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            format!("// Datastar v{VERSION}"),
            "assets/datastar.js opens with {first:?}; run scripts/datastar.sh {VERSION}"
        );
    }
}
