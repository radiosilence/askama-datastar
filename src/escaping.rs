//! A guard for an application's tests: every answer escapes text exactly once.
//!
//! Two mistakes look alike and break a page in opposite ways. A fragment
//! spliced into a template as text, rather than as [`Html`](crate::Html), comes
//! out escaped, and the page shows its markup as raw HTML. Text spliced in as
//! markup comes out unescaped, and runs. Neither is caught by a test that
//! checks a page with ordinary text in it.
//!
//! The guard checks answers drawn with markup-shaped text, the hostile text,
//! in every field a person or a peer controls. An application's test seeds
//! that text, requests every page (whole and in place) and every stream
//! through its router with [`get`] (or [`send`], for a request carrying a
//! session), and asks [`Guard::problems`] of each answer:
//!
//! - No markup of the application's own comes out escaped: `&#60;` or `&lt;`
//!   followed by an element name its templates write.
//! - The hostile text is never escaped twice (`&#38;#60;`, `&amp;lt;`).
//! - The hostile text is never left as markup, outside the `data: signals`
//!   lines of an event stream, which are JSON rather than markup.
//!
//! Where a page shows the text, the test checks it is there escaped once:
//! `answer.contains(&escaped(HOSTILE))`.
//!
//! ```no_run
//! # async fn test(router: axum::Router) -> std::io::Result<()> {
//! use std::time::Duration;
//! use askama_datastar::escaping::{Guard, escaped, get};
//!
//! const HOSTILE: &str = r#"<em>hostile "q" & 'x'</em>"#;
//! let guard = Guard::new(&[HOSTILE]).templates(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))?;
//! // seed HOSTILE into every field a person controls, then:
//! for in_place in [false, true] {
//!     let (status, answer) = get(&router, "/notes", in_place, Duration::from_secs(2)).await;
//!     assert!(status.is_success());
//!     assert_eq!(guard.problems(&answer), []);
//!     assert!(answer.contains(&escaped(HOSTILE)));
//! }
//! # Ok(()) }
//! ```

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt as _;
use tower_service::Service as _;

use crate::request::HEADER;

/// Checks answers for text escaped other than once.
///
/// It knows the hostile texts and the element names the application's own
/// markup writes, gathered from wherever that markup lives: a templates
/// directory ([`Guard::templates`]), markup in hand ([`Guard::markup`]), or
/// names ([`Guard::elements`]).
#[derive(Clone, Debug)]
pub struct Guard {
    /// The element names the application's markup writes.
    elements: BTreeSet<String>,
    /// The element names the hostile texts write, which come out escaped by
    /// right.
    hostile: BTreeSet<String>,
    /// Each hostile text as far as an attribute escaper would leave it alone:
    /// up to its first quote or ampersand.
    unescaped: Vec<String>,
    /// The forms text takes escaped twice.
    twice: Vec<String>,
}

impl Guard {
    /// A guard for answers drawn with `hostile` in the fields people control,
    /// knowing no element names yet.
    ///
    /// Each hostile text starts with an element, as in
    /// `<em>hostile "q" & 'x'</em>`, whose name the application's markup
    /// should not write itself: an escaped element of that name is taken for
    /// the hostile text's own. Holding `"`, `'` and `&` too, it tests every
    /// character escaping changes.
    ///
    /// # Panics
    ///
    /// If a hostile text does not start with an element.
    #[must_use]
    pub fn new(hostile: &[&str]) -> Self {
        let mut names = BTreeSet::new();
        let mut unescaped = Vec::with_capacity(hostile.len());
        for text in hostile {
            let start = text.split(['"', '\'', '&']).next().unwrap_or_default();
            assert!(
                start.starts_with('<') && element_names(start).next().is_some(),
                "{text:?} does not start with an element"
            );
            names.extend(element_names(text));
            unescaped.push(start.to_owned());
        }
        let mut twice: Vec<String> = ['<', '>', '"', '\'', '&']
            .iter()
            .map(|c| escaped(&escaped(&c.to_string())))
            .collect();
        twice.extend(
            ["&amp;lt;", "&amp;gt;", "&amp;quot;", "&amp;#", "&amp;amp;"].map(String::from),
        );
        Self {
            elements: BTreeSet::new(),
            hostile: names,
            unescaped,
            twice,
        }
    }

    /// The element names written by every text file under `dir` and its
    /// subdirectories, as askama's templates directory holds them. A file that
    /// is not UTF-8, such as an image or `.DS_Store`, is passed over.
    ///
    /// # Errors
    ///
    /// If a directory or file under it cannot be read.
    pub fn templates(mut self, dir: impl AsRef<Path>) -> io::Result<Self> {
        let mut dirs = vec![dir.as_ref().to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if let Ok(text) = String::from_utf8(std::fs::read(&path)?) {
                    self.elements.extend(element_names(&text));
                }
            }
        }
        Ok(self)
    }

    /// The element names `markup` writes: a template's source, or the markup
    /// a component library renders.
    #[must_use]
    pub fn markup(mut self, markup: &str) -> Self {
        self.elements.extend(element_names(markup));
        self
    }

    /// Element names the application's markup writes, given by name.
    #[must_use]
    pub fn elements<'a>(mut self, names: impl IntoIterator<Item = &'a str>) -> Self {
        self.elements
            .extend(names.into_iter().map(str::to_ascii_lowercase));
        self
    }

    /// What is wrong with `answer`: a page, an event stream, or a fragment.
    /// Empty when nothing is.
    #[must_use]
    pub fn problems(&self, answer: &str) -> Vec<Problem> {
        let mut out = Vec::new();
        for open in ["&#60;", "&lt;"] {
            for (at, _) in answer.match_indices(open) {
                let rest = answer[at + open.len()..].trim_start_matches('/');
                if element_name(rest).is_some_and(|name| {
                    self.elements.contains(&name) && !self.hostile.contains(&name)
                }) {
                    out.push(Problem::EscapedMarkup(excerpt(answer, at)));
                }
            }
        }
        for form in &self.twice {
            if let Some(at) = answer.find(form.as_str()) {
                out.push(Problem::EscapedTwice(excerpt(answer, at)));
            }
        }
        let markup: String = answer
            .lines()
            .filter(|line| !line.starts_with("data: signals"))
            .collect::<Vec<_>>()
            .join("\n");
        for text in &self.unescaped {
            if let Some(at) = markup.find(text.as_str()) {
                out.push(Problem::Unescaped(excerpt(&markup, at)));
            }
        }
        out
    }
}

/// Something an answer escaped other than once, with the text around it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Problem {
    /// Markup of the templates' own came out escaped: a fragment spliced in as
    /// text, which shows the page its markup as raw HTML.
    EscapedMarkup(String),
    /// Text was escaped twice: escaped markup escaped again as text.
    EscapedTwice(String),
    /// A hostile text was left as markup, which runs.
    Unescaped(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EscapedMarkup(at) => write!(f, "markup escaped: {at}"),
            Self::EscapedTwice(at) => write!(f, "escaped twice: {at}"),
            Self::Unescaped(at) => write!(f, "not escaped: {at}"),
        }
    }
}

/// `text` as askama's HTML escaper writes it, which is how a page shows text
/// escaped once.
#[must_use]
pub fn escaped(text: &str) -> String {
    match askama::filters::escape(text, askama::filters::Html) {
        Ok(escaped) => escaped.to_string(),
        Err(never) => match never {},
    }
}

/// What `router` answers to a GET of `address`, as a browser loading it
/// (`in_place` false) or as Datastar asking (`in_place` true), read for at
/// most `wait`. A live stream does not end by itself, so it is cut off there,
/// having sent what it draws first.
///
/// # Panics
///
/// If `address` is not a valid URI.
pub async fn get(
    router: &Router,
    address: &str,
    in_place: bool,
    wait: Duration,
) -> (StatusCode, String) {
    let mut request = Request::get(address);
    if in_place {
        request = request.header(HEADER, "true");
    }
    let request = request.body(Body::empty()).expect("a GET of a valid URI");
    send(router, request, wait).await
}

/// What `router` answers to `request`, read for at most `wait`: [`get`] for a
/// request it cannot build, such as one carrying a session's cookie, a host,
/// or a form's body.
///
/// ```no_run
/// # async fn test(router: axum::Router) {
/// use std::time::Duration;
/// use axum::body::Body;
/// use axum::http::Request;
///
/// let request = Request::get("/notes")
///     .header("cookie", "session=s3cret")
///     .header("datastar-request", "true")
///     .body(Body::empty())
///     .unwrap();
/// let (status, answer) = askama_datastar::escaping::send(&router, request, Duration::from_secs(2)).await;
/// # }
/// ```
pub async fn send(router: &Router, request: Request<Body>, wait: Duration) -> (StatusCode, String) {
    let response = match router.clone().call(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    let status = response.status();
    let mut body = response.into_body().into_data_stream();
    let mut text = Vec::new();
    let ends = tokio::time::Instant::now() + wait;
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(ends, body.next()).await {
        text.extend_from_slice(&chunk);
    }
    (status, String::from_utf8_lossy(&text).into_owned())
}

/// The element names `markup` writes, lower case.
fn element_names(markup: &str) -> impl Iterator<Item = String> {
    markup
        .split('<')
        .skip(1)
        .filter_map(|part| element_name(part.trim_start_matches('/')))
}

/// The element name `text` starts with, lower case.
fn element_name(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        .then(|| name.to_ascii_lowercase())
}

/// Up to 80 bytes of `text` from `at`.
fn excerpt(text: &str, at: usize) -> String {
    let mut end = (at + 80).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    text[at..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{element_names, escaped};

    #[test]
    fn element_names_are_read_from_markup() {
        let names: Vec<String> =
            element_names(r#"<!doctype html><P class="x">{% if a < b %}</p><my-card/>"#).collect();
        assert_eq!(names, ["p", "p", "my-card"]);
    }

    #[test]
    fn askamas_escaper_writes_numeric_references() {
        assert_eq!(escaped(r#"<a "&'>"#), "&#60;a &#34;&#38;&#39;&#62;");
    }
}
