//! Pages drawn whole for a browser or in place for the page itself.
//!
//! An application whose pages each have an address loads a page two ways: a
//! browser asks for the whole document, and the page, following a link in
//! place, asks Datastar to fetch the address and morph the new body into the
//! one it shows. Both answers are drawn from one body, so a reload, a shared
//! link, back and forward all show what the address names.
//!
//! Each piece here works alone:
//!
//! - [`View`] reads an address's query leniently, as an extractor or from a
//!   string.
//! - [`address`] is a request's address as the person sees it, less what
//!   Datastar adds.
//! - [`history`] writes the attributes that keep the address bar on a signal
//!   holding the address.
//! - [`go`] is a patch that loads an address whole.
//!
//! [`Page`], [`Layout`] and [`serve`] put them together: the application
//! writes its document once, as a [`Layout`], and each page as a [`Page`], and
//! [`serve`] is the handler for every page.

use std::future::Future;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response};
use datastar::prelude::{ElementPatchMode, PatchElements, PatchSignals};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{Html, events};

/// The document an application's pages are drawn in.
///
/// It decides everything around a page's body; [`serve`] decides which
/// answer the request gets. One type implements it for an application, or
/// one for each kind of document.
pub trait Layout: 'static {
    /// The application's state, as axum's router holds it.
    type State: Clone + Send + Sync + 'static;
    /// What a page tells the layout about itself beyond its body: which entry
    /// of a menu to mark, which live stream to hold open. `()` when nothing.
    type Place: Sync + 'static;
    /// Why a page could not be drawn.
    type Error: Send;

    /// The selector of the element a body is morphed into in place. The
    /// whole document writes the body inside it.
    const MAIN: &'static str = "#main";
    /// The signal holding the page's address. A stream that draws the page an
    /// address names reads it, and [`history`] keeps the address bar on it.
    const ADDRESS: &'static str = "url";

    /// The whole document around `page`, for a browser loading the address.
    fn whole(
        state: &Self::State,
        place: &Self::Place,
        page: Whole,
    ) -> impl Future<Output = Result<Html, Self::Error>> + Send;

    /// The events for the parts of the document outside [`Layout::MAIN`] that
    /// change with the page, such as its title, sent beside the body when it
    /// loads in place. `title` is the page's own, when it gives one.
    fn in_place(
        state: &Self::State,
        place: &Self::Place,
        title: Option<&str>,
    ) -> impl Future<Output = Vec<Event>> + Send;

    /// The answer for a page that failed, whole for a browser or in place.
    /// In place, a status alone would leave the last page showing with no
    /// word of why, so this is the layout's to say.
    fn failed(state: &Self::State, error: Self::Error, in_place: bool) -> Response;
}

/// A page: where it sits in its layout, what it reads from the request, and
/// its body.
pub trait Page: Send + 'static {
    /// The document the page is drawn in.
    type Layout: Layout;
    /// What the page tells its layout about itself.
    const PLACE: <Self::Layout as Layout>::Place;
    /// What the page reads from the request: any axum extractor that leaves
    /// the body alone, such as [`View`] for the query or `Path` for an id.
    type Args: FromRequestParts<<Self::Layout as Layout>::State, Rejection: IntoResponse> + Send;

    /// The page's body.
    fn body(
        state: &<Self::Layout as Layout>::State,
        args: Self::Args,
    ) -> impl Future<Output = Result<Body, <Self::Layout as Layout>::Error>> + Send;
}

/// A page's body, with what its layout needs to know of it.
#[derive(Debug)]
pub struct Body {
    html: Html,
    status: StatusCode,
    title: Option<String>,
    signals: Map<String, Value>,
    redirect: Option<String>,
}

impl Body {
    /// The page's markup, answered with 200.
    #[must_use]
    pub fn new(html: Html) -> Self {
        Self {
            html,
            status: StatusCode::OK,
            title: None,
            signals: Map::new(),
            redirect: None,
        }
    }

    /// The address now names the page at the path `to`: a browser is
    /// redirected there, and a page loading in place goes there with [`go`].
    ///
    /// # Panics
    ///
    /// If `to` is not a path on this origin, as [`go`] does.
    #[must_use]
    pub fn redirect(to: impl Into<String>) -> Self {
        let to = to.into();
        assert_on_origin(&to);
        Self {
            redirect: Some(to),
            ..Self::new(Html::new())
        }
    }

    /// The page's own title, for its layout to show. An empty one is none.
    #[must_use]
    pub fn title(mut self, title: impl Into<Option<String>>) -> Self {
        self.title = title.into().filter(|t| !t.is_empty());
        self
    }

    /// The status a browser loading the address gets, such as 404 for a page
    /// naming something that is not there. In place, the answer is events
    /// whatever it is.
    #[must_use]
    pub fn status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// A signal of the page's own, set as the page is shown. The layout's
    /// [`Layout::ADDRESS`] signal is set after these, so it is always the
    /// address.
    #[must_use]
    pub fn signal(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.signals.insert(name.to_owned(), value.into());
        self
    }
}

impl From<Html> for Body {
    fn from(html: Html) -> Self {
        Self::new(html)
    }
}

/// A page as its layout draws the whole document around it.
#[derive(Debug)]
#[non_exhaustive]
pub struct Whole {
    /// The page's body, which the document writes inside [`Layout::MAIN`].
    pub body: Html,
    /// The page's own title, when it gives one.
    pub title: Option<String>,
    /// The page's signals, its address among them. A layout with signals of
    /// its own adds them before writing [`Whole::attributes`].
    pub signals: Map<String, Value>,
    /// The name of the signal holding the address.
    pub address: &'static str,
}

impl Whole {
    /// The attributes of the element holding the page's signals, normally
    /// `<body>`: `data-signals` with [`Whole::signals`], and [`history`] on
    /// the address signal. The text starts with a space:
    /// `<body{{ attributes }}>`.
    #[must_use]
    pub fn attributes(&self) -> Html {
        let json = Value::Object(self.signals.clone()).to_string();
        let mut out = String::from(" data-signals=\"");
        escape_attribute(&json, &mut out);
        out.push('"');
        out.push_str(history(self.address).as_str());
        Html::trusted(out)
    }
}

/// Attributes that keep the address bar on the address in `signal`: when the
/// signal changes, as a page loaded in place sets it, the address is pushed
/// onto the history, and going back or forward to an address the page does
/// not show loads that address whole. The text starts with a space.
///
/// ```
/// let attributes = askama_datastar::page::history("url");
/// assert!(attributes.contains("history.pushState(null, '', $url)"));
/// ```
///
/// The address is read from the signal, never written into the expression.
///
/// # Panics
///
/// If `signal` is not letters, digits and `_`: it is written into the
/// expressions, so it is the program's own text.
#[must_use]
pub fn history(signal: &str) -> Html {
    assert!(
        !signal.is_empty()
            && signal
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "{signal:?} is not a signal name"
    );
    let at = "location.pathname + location.search";
    Html::trusted(format!(
        " data-effect=\"${signal} && ${signal} != {at} && history.pushState(null, '', ${signal})\" \
         data-on:popstate__window=\"{at} != ${signal} && location.reload()\""
    ))
}

/// The view an address's query describes. A query that does not parse, such
/// as a value of the wrong type, is `T::default()` rather than an error: an
/// address is something a person types, edits and keeps, and the default view
/// answers a stale one better than 400 does. A key may repeat, filling a
/// `Vec`, as a form's checkboxes send it.
///
/// [`Signals`](crate::Signals) is the same for the signals a request carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View<T>(pub T);

impl<T: DeserializeOwned + Default> View<T> {
    /// The view the query of `address` (a path and query, or a whole URL)
    /// describes, as a stream reads it from the signal holding its page's
    /// address. Datastar's `datastar` parameter is not part of it.
    #[must_use]
    pub fn of(address: &str) -> Self {
        let query = address.split_once('?').map_or("", |(_, q)| q);
        let query = query.split_once('#').map_or(query, |(q, _)| q);
        Self(serde_html_form::from_str(&without_datastar(query)).unwrap_or_default())
    }
}

impl<T: DeserializeOwned + Default + Send, S: Sync> FromRequestParts<S> for View<T> {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self::of(&parts.uri.to_string())))
    }
}

/// A request's address as the person sees it: its path and query, less the
/// `datastar` parameter Datastar adds to a GET it sends. The other parameters
/// are kept as they were written.
///
/// ```
/// let uri = "/notes?q=moss&datastar=%7B%7D".parse().unwrap();
/// assert_eq!(askama_datastar::page::address(&uri), "/notes?q=moss");
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

/// Draws page `P`: whole for a browser, or in place for the page itself.
/// A handler: `get(page::serve::<Notes>)`.
pub async fn serve<P: Page>(
    State(state): State<<P::Layout as Layout>::State>,
    headers: HeaderMap,
    uri: Uri,
    args: P::Args,
) -> Response {
    let in_place = crate::is_datastar(&headers);
    let body = match P::body(&state, args).await {
        Ok(body) => body,
        Err(error) => return P::Layout::failed(&state, error, in_place),
    };
    let url = address(&uri);
    if in_place {
        return events(shown::<P>(&state, body, &url).await);
    }
    if let Some(to) = body.redirect {
        return Redirect::to(&to).into_response();
    }
    let mut signals = body.signals;
    signals.insert(P::Layout::ADDRESS.to_owned(), url.into());
    let whole = Whole {
        body: body.html,
        title: body.title,
        signals,
        address: P::Layout::ADDRESS,
    };
    match P::Layout::whole(&state, &P::PLACE, whole).await {
        Ok(html) => (body.status, html).into_response(),
        Err(error) => P::Layout::failed(&state, error, false),
    }
}

/// The events that show page `P`'s `body` in place at `address`: the body
/// morphed into [`Layout::MAIN`], the layout's [`Layout::in_place`] events,
/// and the page's signals with the address, which moves the address bar. A
/// body that redirects goes to its address instead.
///
/// [`serve`] answers an in-place load with these, and an action that leads to
/// another page answers with them too, so that page is drawn as a load of its
/// address would draw it.
pub async fn shown<P: Page>(
    state: &<P::Layout as Layout>::State,
    body: Body,
    address: &str,
) -> Vec<Event> {
    if let Some(to) = body.redirect {
        return vec![go(&to).into()];
    }
    let mut signals = body.signals;
    signals.insert(P::Layout::ADDRESS.to_owned(), address.into());
    let mut out = vec![
        body.html
            .patch()
            .selector(P::Layout::MAIN)
            .mode(ElementPatchMode::Inner)
            .into(),
    ];
    out.extend(P::Layout::in_place(state, &P::PLACE, body.title.as_deref()).await);
    out.push(PatchSignals::new(Value::Object(signals).to_string()).into());
    out
}

/// A patch that loads the path `to` whole, as following a link does.
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
    assert_on_origin(to);
    let mut markup = String::from("<div hidden data-go=\"");
    escape_attribute(to, &mut markup);
    markup.push_str("\" data-init=\"location.assign(el.dataset.go)\"></div>");
    Html::trusted(markup)
        .patch()
        .selector("body")
        .mode(ElementPatchMode::Append)
}

fn assert_on_origin(path: &str) {
    assert!(
        path.starts_with('/')
            && !path.starts_with("//")
            && !path.starts_with("/\\")
            && !path.chars().any(|c| c.is_control() || c.is_whitespace()),
        "{path:?} is not a path on this origin"
    );
}

fn escape_attribute(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{View, address, without_datastar};

    #[test]
    fn the_address_leaves_out_what_datastar_adds() {
        let uri = "/search?tab=t1&datastar=%7B%22url%22%3A%22%2F%22%7D&q=a%20b"
            .parse()
            .unwrap();
        assert_eq!(address(&uri), "/search?tab=t1&q=a%20b");
        assert_eq!(address(&"/".parse().unwrap()), "/");
        assert_eq!(address(&"/x?datastar=%7B%7D".parse().unwrap()), "/x");
        assert_eq!(without_datastar("data%73tar=1&a=2"), "a=2");
    }

    #[derive(serde::Deserialize, Default, Debug, PartialEq)]
    #[serde(default)]
    struct Filters {
        q: String,
        kind: Vec<String>,
        page: u32,
    }

    #[test]
    fn a_view_reads_repeated_keys_and_falls_back_to_the_default() {
        let View(f) = View::<Filters>::of("/search?q=moss&kind=a&kind=b&page=2#top");
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
    #[should_panic(expected = "is not a signal name")]
    fn the_address_signal_is_the_programs_own() {
        let _ = super::history("url); alert(1");
    }
}
