//! Pages of a routed application, each drawn whole for a browser or in place
//! for the page itself.
//!
//! Every page has one shape, [`Page`]: where it sits in the application, the
//! live stream it holds open, what it reads from the request, and its
//! [`Body`]. The application's [`Layout`] is the document around every body.
//! [`serve`] is the one handler that draws any page:
//!
//! - A browser asking for the address gets the layout around the body.
//! - The page asking for it, through a link or form that loads in place, gets
//!   the body morphed into the layout's main region, the layout's own regions
//!   that change with the page (such as the title), and the address as the
//!   `url` signal, which the address bar follows.
//!
//! So a reload, a shared link, back and forward all draw exactly what the
//! address names, and loading in place draws the same page without a new
//! document. An action that takes the page elsewhere answers the same way,
//! with [`shown`].

use std::future::Future;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response};
use datastar::prelude::{ElementPatchMode, PatchElements, PatchSignals};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{Html, events, reopen_on};

/// The signal holding the page's address, path and query, as the address bar
/// shows it. A page's stream reads it to know what to draw, and the page opens
/// its stream afresh when it changes.
pub const URL: &str = "url";

/// The document every page of an application is drawn in.
///
/// One type implements it for the application, or one for each kind of
/// document if pages differ in their frame. It decides everything around a
/// page's body; [`serve`] decides whether it is drawn at all.
pub trait Layout: 'static {
    /// The application's state, as axum's router holds it.
    type State: Clone + Send + Sync + 'static;
    /// What a page tells the layout about itself beyond its body, such as the
    /// entry of the navigation it is shown under. `()` when nothing.
    type Place: Sync + 'static;
    /// Why a page could not be drawn.
    type Error: Send;

    /// The selector of the element a page's body is morphed into when it
    /// loads in place. The whole document writes the body inside it.
    const MAIN: &'static str = "#main";

    /// The document around `page`, for a browser loading the address.
    fn whole(
        state: &Self::State,
        place: &Self::Place,
        page: Whole,
    ) -> impl Future<Output = Result<Html, Self::Error>> + Send;

    /// The layout's own regions that change with the page, sent beside its
    /// body when it loads in place: the title, a notice of the last page's
    /// cleared. `heading` is what the page calls itself, when it says.
    fn in_place(
        state: &Self::State,
        place: &Self::Place,
        heading: Option<&str>,
    ) -> impl Future<Output = Vec<Event>> + Send;

    /// The answer for a page that failed: whole for a browser, or in place,
    /// where a status alone would leave the person looking at the last page
    /// with no word of why.
    fn failed(state: &Self::State, error: Self::Error, in_place: bool) -> Response;
}

/// A page of an application.
///
/// ```
/// # use askama_datastar::Html;
/// # use askama_datastar::page::{Body, Layout, Page, View, Whole};
/// # use axum::response::{IntoResponse, Response, sse::Event};
/// # #[derive(Clone)] struct App;
/// # struct Frame;
/// # impl Layout for Frame {
/// #     type State = App; type Place = (); type Error = std::convert::Infallible;
/// #     async fn whole(_: &App, (): &(), page: Whole) -> Result<Html, Self::Error> { Ok(page.body) }
/// #     async fn in_place(_: &App, (): &(), _: Option<&str>) -> Vec<Event> { Vec::new() }
/// #     fn failed(_: &App, e: Self::Error, _: bool) -> Response { match e {} }
/// # }
/// #[derive(serde::Deserialize, Default)]
/// #[serde(default)]
/// struct Query {
///     q: String,
/// }
///
/// struct Search;
///
/// impl Page for Search {
///     type Layout = Frame;
///     const PLACE: () = ();
///     const STREAM: &'static str = "/stream/search";
///     type Args = View<Query>;
///
///     async fn body(_: &App, View(query): View<Query>) -> Result<Body, std::convert::Infallible> {
///         Ok(Body::new(Html::trusted("<p>results</p>")).heading(query.q))
///     }
/// }
/// ```
pub trait Page: Send + 'static {
    /// The document the page is drawn in.
    type Layout: Layout;
    /// What the page tells its layout about itself.
    const PLACE: <Self::Layout as Layout>::Place;
    /// The address of the live stream the page holds open.
    const STREAM: &'static str;
    /// What the page reads from the request: any axum extractor that leaves
    /// the body alone, such as [`View`] for the address's query or `Path`.
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
    heading: Option<String>,
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
            heading: None,
            signals: Map::new(),
            redirect: None,
        }
    }

    /// The address now names another page, at the path `to`: a browser is
    /// redirected there, and a page loading in place goes there.
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

    /// What the page calls itself, for the layout's title, when its place
    /// does not say. An empty heading is none.
    #[must_use]
    pub fn heading(mut self, heading: impl Into<Option<String>>) -> Self {
        self.heading = heading.into().filter(|h| !h.is_empty());
        self
    }

    /// The status a browser loading the address is answered with, such as
    /// 404 for a page naming something that is not there. An in-place load
    /// is answered with events whatever it is.
    #[must_use]
    pub fn status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// A signal of the page's own, set as the page is shown.
    ///
    /// # Panics
    ///
    /// If `name` is [`URL`], which [`serve`] sets.
    #[must_use]
    pub fn signal(mut self, name: &str, value: impl Into<Value>) -> Self {
        assert!(name != URL, "the `{URL}` signal is the page's address");
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
    /// What the page calls itself, when it says.
    pub heading: Option<String>,
    /// The page's signals, its address as [`URL`] among them. A layout with
    /// signals of its own adds them before writing [`Whole::attributes`].
    pub signals: Map<String, Value>,
    /// The address of the page's live stream.
    pub stream: &'static str,
}

impl Whole {
    /// The attributes of the element holding the page's signals, normally
    /// `<body>`: `data-signals` with [`Whole::signals`], and the effects that
    /// keep the address bar on the address. An in-place load sets `$url`,
    /// which pushes it onto the history; going back or forward to an address
    /// the page does not show loads it whole.
    ///
    /// The text starts with a space, so it follows the tag's name directly:
    /// `<body{{ attributes }}>`.
    #[must_use]
    pub fn attributes(&self) -> Html {
        let json = Value::Object(self.signals.clone()).to_string();
        let mut out = String::with_capacity(json.len() + HISTORY.len() + 16);
        out.push_str(" data-signals=\"");
        escape_attribute(&json, &mut out);
        out.push('"');
        out.push_str(HISTORY);
        Html::trusted(out)
    }

    /// The attributes that hold the page's live stream open, for an element of
    /// their own: [`reopen_on`] the stream, following [`URL`] and `also`, the
    /// layout's signals its streams read.
    ///
    /// # Panics
    ///
    /// As [`reopen_on`] does.
    #[must_use]
    pub fn live(&self, also: &[&'static str]) -> Html {
        let mut signals = Vec::with_capacity(also.len() + 1);
        signals.push(URL);
        signals.extend_from_slice(also);
        reopen_on(self.stream, &signals)
    }
}

/// The effects that keep the address bar on `$url`. Constant: the address is
/// read from the signal, never written into an expression.
const HISTORY: &str = concat!(
    r#" data-effect="$url && $url != location.pathname + location.search && history.pushState(null, '', $url)""#,
    r#" data-on:popstate__window="location.pathname + location.search != $url && location.reload()""#,
);

/// The view an address's query describes. One that does not parse, such as a
/// value of the wrong type, is `T::default()` rather than an error: an address
/// is something a person types, edits and keeps, and the default view is a
/// better answer to a stale one than 400. A key may repeat, filling a `Vec`,
/// as a form's checkboxes send it.
///
/// [`Signals`](crate::Signals) is the same for the signals a Datastar request
/// carries. A stream reads the view of the page it serves from the `url`
/// signal with [`View::of`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View<T>(pub T);

impl<T: DeserializeOwned + Default> View<T> {
    /// The view the query of `address` (a path and query, or a whole URL)
    /// describes. Datastar's own `datastar` parameter is not part of it.
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

/// `query` without the `datastar` parameter Datastar adds to a GET it sends,
/// the other parameters as they were written.
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

/// The page's address as the person sees it: the request's path and query,
/// less what Datastar adds.
fn address(uri: &Uri) -> String {
    let query = without_datastar(uri.query().unwrap_or_default());
    if query.is_empty() {
        uri.path().to_owned()
    } else {
        format!("{}?{query}", uri.path())
    }
}

/// Draws page `P`: whole for a browser, or in place for the page itself.
///
/// ```
/// # use askama_datastar::Html;
/// # use askama_datastar::page::{self, Body, Layout, Page, Whole};
/// # use axum::response::{Response, sse::Event};
/// # #[derive(Clone)] struct App;
/// # struct Frame;
/// # impl Layout for Frame {
/// #     type State = App; type Place = (); type Error = std::convert::Infallible;
/// #     async fn whole(_: &App, (): &(), page: Whole) -> Result<Html, Self::Error> { Ok(page.body) }
/// #     async fn in_place(_: &App, (): &(), _: Option<&str>) -> Vec<Event> { Vec::new() }
/// #     fn failed(_: &App, e: Self::Error, _: bool) -> Response { match e {} }
/// # }
/// # struct Settings;
/// # impl Page for Settings {
/// #     type Layout = Frame; const PLACE: () = (); const STREAM: &'static str = "/stream";
/// #     type Args = ();
/// #     async fn body(_: &App, (): ()) -> Result<Body, std::convert::Infallible> { Ok(Body::new(Html::new())) }
/// # }
/// let app: axum::Router = axum::Router::new()
///     .route("/settings", axum::routing::get(page::serve::<Settings>))
///     .with_state(App);
/// ```
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
    signals.insert(URL.to_owned(), url.into());
    let whole = Whole {
        body: body.html,
        heading: body.heading,
        signals,
        stream: P::STREAM,
    };
    match P::Layout::whole(&state, &P::PLACE, whole).await {
        Ok(html) => (body.status, html).into_response(),
        Err(error) => P::Layout::failed(&state, error, false),
    }
}

/// The events that show page `P`'s `body` in place at the address `url`: the
/// body morphed into [`Layout::MAIN`], the layout's [`Layout::in_place`]
/// regions, and the page's signals with `url` as [`URL`], which moves the
/// address bar. A body that redirects goes to its address instead.
///
/// [`serve`] answers an in-place load with these, and an action that takes
/// the page somewhere answers with them too, so the page it leads to is drawn
/// as a load of its address would draw it.
pub async fn shown<P: Page>(
    state: &<P::Layout as Layout>::State,
    body: Body,
    url: &str,
) -> Vec<Event> {
    if let Some(to) = body.redirect {
        return vec![go(&to).into()];
    }
    let mut signals = body.signals;
    signals.insert(URL.to_owned(), url.into());
    let mut out = vec![
        body.html
            .patch()
            .selector(P::Layout::MAIN)
            .mode(ElementPatchMode::Inner)
            .into(),
    ];
    out.extend(P::Layout::in_place(state, &P::PLACE, body.heading.as_deref()).await);
    out.push(PatchSignals::new(Value::Object(signals).to_string()).into());
    out
}

/// A patch that loads the page at the path `to` whole, as following a link
/// does: what an action answers when the page it leads to is better loaded
/// afresh than drawn in place.
///
/// It appends a hidden element to the body that loads the address once it is
/// set up. The address is written into its `data-go` attribute, escaped, and
/// read with `el.dataset.go`, never into an expression.
///
/// # Panics
///
/// If `to` is not a path on this origin: it must start with `/`, and not
/// with `//` or `/\`, nor hold a control or space character, which a browser
/// may read as another host.
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

/// `text` escaped as an attribute value.
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
}
