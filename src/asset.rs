//! Files embedded in the binary, served under an address that names their
//! content.
//!
//! A page names each asset with a hash of its bytes in the query
//! (`/assets/app.css?v=…`), so a page from a new build fetches the new file
//! rather than a cached copy of the last, and the file at that address can be
//! cached for good. Asked for any other way, by a bare path or a page from an
//! earlier build, it is cached for an hour, since that address will not always
//! be these bytes.

use std::fmt;
use std::future::{Ready, ready};
use std::sync::OnceLock;

use axum::extract::RawQuery;
use axum::http::header;
use axum::response::{IntoResponse, Response};

/// A file embedded in the binary, with its content type.
///
/// `const`, so a `static` holds it, and the hash of its bytes is worked out
/// once, the first time it is asked for:
///
/// ```
/// use askama_datastar::Asset;
/// use axum::Router;
/// use axum::routing::get;
///
/// static APP_CSS: Asset = Asset::new(b"main { margin: 0 }", "text/css; charset=utf-8");
///
/// let app: Router = Router::new().route("/assets/app.css", get(APP_CSS.serve()));
///
/// // What the page's `<link rel="stylesheet" href="…">` names.
/// let href = APP_CSS.url("/assets/app.css");
/// assert!(href.starts_with("/assets/app.css?v="));
/// ```
pub struct Asset {
    bytes: &'static [u8],
    content_type: &'static str,
    /// The hash of `bytes`, in hex: the `v` parameter that names this content.
    hash: OnceLock<String>,
}

impl Asset {
    /// `bytes`, served as `content_type`: `text/css; charset=utf-8`,
    /// `text/javascript; charset=utf-8`, `font/woff2`, `image/svg+xml`.
    #[must_use]
    pub const fn new(bytes: &'static [u8], content_type: &'static str) -> Self {
        Self {
            bytes,
            content_type,
            hash: OnceLock::new(),
        }
    }

    /// `path` with the parameter naming these bytes, `path?v=<hash>`, for the
    /// page's `href` or `src`; `&v=<hash>` when `path` has a query already. The
    /// hash is FNV-1a over the bytes, 64 bits in hex: enough to notice a
    /// change, which is all a fingerprint is for.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        let join = if path.contains('?') { '&' } else { '?' };
        format!("{path}{join}v={}", self.hash())
    }

    /// The file, cached for a year as immutable when `query`'s `v` parameter
    /// is the hash [`url`](Self::url) gives, and for an hour otherwise. For a
    /// handler that serves several assets from one route:
    ///
    /// ```
    /// use askama_datastar::Asset;
    /// use axum::extract::{Path, RawQuery};
    /// use axum::http::StatusCode;
    /// use axum::response::{IntoResponse, Response};
    ///
    /// static APP_CSS: Asset = Asset::new(b"main { margin: 0 }", "text/css; charset=utf-8");
    /// static APP_JS: Asset = Asset::new(b"export {};", "text/javascript; charset=utf-8");
    ///
    /// async fn assets(Path(name): Path<String>, RawQuery(query): RawQuery) -> Response {
    ///     match name.as_str() {
    ///         "app.css" => APP_CSS.response(query.as_deref()),
    ///         "app.js" => APP_JS.response(query.as_deref()),
    ///         _ => StatusCode::NOT_FOUND.into_response(),
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn response(&self, query: Option<&str>) -> Response {
        let named = query.is_some_and(|query| {
            form_urlencoded::parse(query.as_bytes())
                .find(|(name, _)| name == "v")
                .is_some_and(|(_, hash)| hash == self.hash())
        });
        let cache = if named {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        };
        (
            [
                (header::CONTENT_TYPE, self.content_type),
                (header::CACHE_CONTROL, cache),
            ],
            self.bytes,
        )
            .into_response()
    }

    /// The file as a handler, for the route [`url`](Self::url)'s `path` names:
    /// `get(APP_CSS.serve())`.
    pub fn serve(&'static self) -> impl FnOnce(RawQuery) -> Ready<Response> + Clone + Send + Sync {
        move |RawQuery(query)| ready(self.response(query.as_deref()))
    }

    fn hash(&self) -> &str {
        self.hash
            .get_or_init(|| format!("{:016x}", fnv1a(self.bytes)))
    }
}

/// The content type and size; the bytes would fill a log.
impl fmt::Debug for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Asset")
            .field("content_type", &self.content_type)
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// FNV-1a, 64-bit.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::fnv1a;

    #[test]
    fn the_fingerprint_is_fnv1a() {
        // The published 64-bit FNV-1a test vectors.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
