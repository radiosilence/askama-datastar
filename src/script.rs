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
//! loads it needs `'unsafe-eval'` in its `script-src`.

use std::sync::LazyLock;

use axum::extract::RawQuery;
use axum::http::header;
use axum::response::{IntoResponse, Response};

/// The Datastar release vendored.
pub const VERSION: &str = "1.0.5";

/// The script, as that release publishes it.
pub const SOURCE: &str = include_str!("../assets/datastar.js");

/// `v=` and a hash of [`SOURCE`]: the query that names this copy.
static QUERY: LazyLock<String> = LazyLock::new(|| format!("v={:016x}", fnv1a(SOURCE.as_bytes())));

/// `path` with the query naming this copy of the script, for a page's
/// `<script type="module" src="…">`: `/assets/datastar.js?v=…`.
///
/// The query changes whenever the script does, so a page from a new build
/// fetches its own script rather than a cached copy of the last one, and
/// [`serve`] can let a browser keep it for good.
#[must_use]
pub fn url(path: &str) -> String {
    format!("{path}?{}", *QUERY)
}

/// The script, as a handler for the route [`url`]'s `path` names.
///
/// Asked for by the URL [`url`] gives, it is cached for a year as immutable.
/// Asked for any other way, by a bare path or a page from an earlier build, it
/// is cached for an hour, since that address will not always be this copy.
pub async fn serve(RawQuery(query): RawQuery) -> Response {
    let cache = if query.as_deref() == Some(QUERY.as_str()) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, cache),
        ],
        SOURCE,
    )
        .into_response()
}

/// FNV-1a, 64-bit: enough to notice a change, and the hash the pages that
/// served their own copy used, so their URLs for a given copy stay the same.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
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
