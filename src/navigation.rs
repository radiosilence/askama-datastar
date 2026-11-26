//! The address bar and location, for a page that loads in place.
//!
//! A page that follows a link with `@get` keeps its document and morphs in
//! what the server sends. These keep its address right while it does: the
//! address a Datastar request was for, the history following a signal holding
//! it, and a page sent to another address whole. [`View`] reads the view an
//! address's query describes.

use std::future::Future;

use axum::extract::FromRequestParts;
use axum::http::Uri;
use axum::http::request::Parts;
use datastar::prelude::{ElementPatchMode, PatchElements};
use serde::de::DeserializeOwned;

use crate::Html;
use crate::attr;

/// The address a request was for, as the person sees it: its path and query,
/// less the `datastar` parameter in which Datastar sends signals with a GET.
/// The other parameters are kept as they were written.
///
/// Give it axum's `OriginalUri`, not `Uri`: under `Router::nest`, `Uri` has
/// lost the prefix the address bar shows.
///
/// ```
/// use axum::extract::OriginalUri;
///
/// async fn page(OriginalUri(uri): OriginalUri) -> String {
///     askama_datastar::address(&uri)
/// }
///
/// let uri = "/notes?q=moss&datastar=%7B%7D".parse().unwrap();
/// assert_eq!(askama_datastar::address(&uri), "/notes?q=moss");
/// ```
#[must_use]
pub fn address(uri: &Uri) -> String {
    let query = without_datastar(uri.query().unwrap_or_default());
    if query.is_empty() {
        uri.path().to_owned()
    } else {
        format!("{}?{query}", uri.path())
    }
}

/// `query` less its `datastar` parameters, the rest as they were written.
fn without_datastar(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| {
            !pair.is_empty() && {
                let name = pair.split_once('=').map_or(*pair, |(name, _)| name);
                form_urlencoded::parse(name.as_bytes())
                    .next()
                    .is_none_or(|(name, _)| name != "datastar")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// The view an address's query describes. A query that does not parse, such
/// as a value of the wrong type, is `T::default()` rather than an error: an
/// address is something a person types, edits and keeps, and the default view
/// answers a stale one better than 400 does. A key may repeat, filling a
/// `Vec`, as a form's checkboxes send it. Datastar's `datastar` parameter is
/// not part of it.
///
/// [`Signals`](crate::Signals) is the same for the signals a request carries.
///
/// ```
/// use askama_datastar::View;
///
/// #[derive(serde::Deserialize, Default)]
/// #[serde(default)]
/// struct Search {
///     q: String,
///     page: u32,
/// }
///
/// async fn search(View(search): View<Search>) -> String {
///     search.q
/// }
///
/// assert_eq!(View::<Search>::of("/search?q=moss&page=2").0.q, "moss");
/// assert_eq!(View::<Search>::of("/search?q=moss&page=two").0.q, "");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View<T>(pub T);

impl<T: DeserializeOwned + Default> View<T> {
    /// The view the query of `address` (a path and query, or a whole URL)
    /// describes: how a live stream reads the view of its page from the
    /// signal holding the page's address.
    #[must_use]
    pub fn of(address: &str) -> Self {
        let query = address.split_once('?').map_or("", |(_, q)| q);
        Self::from_query(query.split_once('#').map_or(query, |(q, _)| q))
    }

    fn from_query(query: &str) -> Self {
        Self(serde_html_form::from_str(&without_datastar(query)).unwrap_or_default())
    }
}

impl<T: DeserializeOwned + Default + Send, S: Sync> FromRequestParts<S> for View<T> {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self::from_query(parts.uri.query().unwrap_or_default())))
    }
}

/// Attributes that keep the address bar on the address in `signal`. When the
/// signal changes, as an answer loading a page in place sets it, the address
/// is pushed onto the history; going back or forward to an address the page
/// does not show loads that address whole. For the element holding the
/// signal, normally `<body>`; the text starts with a space.
///
/// ```
/// let attributes = askama_datastar::history("url");
/// assert!(attributes.contains("history.pushState(null, '', $url)"));
/// ```
///
/// The address is read from the signal, never written into an expression.
///
/// # Panics
///
/// If `signal` is not letters, digits, `_` and `.`: it is written into the
/// expressions, so it is the program's own text.
#[must_use]
pub fn history(signal: &str) -> Html {
    attr::assert_signal_name(signal);
    let at = "location.pathname + location.search";
    Html::trusted(format!(
        " data-effect=\"${signal} && ${signal} != {at} && history.pushState(null, '', ${signal})\" \
         data-on:popstate__window=\"{at} != ${signal} && location.reload()\""
    ))
}

/// A patch that loads the path `to` whole, as following a link does: the
/// answer in place where a browser's request would be redirected, since
/// Datastar follows a redirect as a fetch and morphs in whatever it finds.
///
/// It appends a hidden element to the body that loads the address once
/// Datastar sets it up. The address is written into its `data-go` attribute,
/// escaped, and read with `el.dataset.go`, never written into an expression.
///
/// # Panics
///
/// If `to` is not a path on this origin: it must start with `/`, not with
/// `//` or `/\`, and hold no control or space character, any of which a
/// browser may read as another host.
#[must_use]
pub fn go(to: &str) -> PatchElements {
    attr::assert_on_origin(to);
    let mut markup = String::from("<div hidden data-go=\"");
    attr::escape(to, &mut markup);
    markup.push_str("\" data-init=\"location.assign(el.dataset.go)\"></div>");
    Html::trusted(markup)
        .patch()
        .selector("body")
        .mode(ElementPatchMode::Append)
}

#[cfg(test)]
mod tests {
    use super::{View, address, go, history};

    #[derive(serde::Deserialize, Default, Debug, PartialEq)]
    #[serde(default)]
    struct Filters {
        q: String,
        kind: Vec<String>,
        page: u32,
    }

    #[test]
    fn a_view_reads_repeated_keys_and_falls_back_to_the_default() {
        let View(f) =
            View::<Filters>::of("/search?q=moss&kind=a&kind=b&page=2&datastar=%7B%7D#top");
        assert_eq!(
            f,
            Filters {
                q: "moss".into(),
                kind: vec!["a".into(), "b".into()],
                page: 2
            }
        );
        assert_eq!(
            View::<Filters>::of("/search?page=two").0,
            Filters::default()
        );
        assert_eq!(View::<Filters>::of("/search").0, Filters::default());
    }

    #[test]
    fn the_address_leaves_out_what_datastar_adds() {
        let uri = "/search?tab=t1&datastar=%7B%22url%22%3A%22%2F%22%7D&q=a%20b"
            .parse()
            .unwrap();
        assert_eq!(address(&uri), "/search?tab=t1&q=a%20b");
        assert_eq!(address(&"/".parse().unwrap()), "/");
        assert_eq!(address(&"/x?datastar=%7B%7D".parse().unwrap()), "/x");
        assert_eq!(address(&"/x?data%73tar=1&a=2".parse().unwrap()), "/x?a=2");
    }

    #[test]
    #[should_panic(expected = "is not a signal name")]
    fn the_address_signal_is_the_programs_own() {
        let _ = history("url); alert(1");
    }

    #[test]
    #[should_panic(expected = "is not a path on this origin")]
    fn a_page_goes_only_to_a_path_on_its_own_origin() {
        let _ = go("//evil.example/");
    }
}
