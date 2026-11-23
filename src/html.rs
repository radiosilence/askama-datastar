//! Markup a template rendered, as a type.
//!
//! A page template escapes every value it is given except one whose type says
//! it is markup already. Making that type here, where a template is rendered,
//! decides what is safe to splice in where the markup is made, rather than by a
//! `|safe` in the template. The same value is what a patch sends, which is what
//! keeps a fragment written once.

use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use askama::Template;
use axum::response::{IntoResponse, Response};
use datastar::prelude::PatchElements;

/// HTML rendered from a template, or written by the program's own code: never
/// text that came from anyone.
///
/// A template embeds an `Html` as it is, since it implements askama's
/// [`HtmlSafe`](askama::filters::HtmlSafe), and escapes everything else:
///
/// ```
/// use askama::Template;
/// use askama_datastar::Html;
///
/// #[derive(Template)]
/// #[template(ext = "html", source = "<p>{{ markup }} {{ text }}</p>")]
/// struct Line {
///     markup: Html,
///     text: String,
/// }
///
/// let line = Line {
///     markup: Html::trusted("<b>kept</b>"),
///     text: "<b>escaped</b>".into(),
/// };
/// assert_eq!(line.render()?, "<p><b>kept</b> &#60;b&#62;escaped&#60;/b&#62;</p>");
/// # Ok::<(), askama::Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Html(String);

impl Html {
    /// No markup, as an absent fragment renders. `const`, so a `static` can
    /// hold one for templates that borrow an `Html` that may not be there.
    #[must_use]
    pub const fn new() -> Self {
        Self(String::new())
    }

    /// Renders `template`. It fails only where a value's `Display` or a filter
    /// does; what to make of that is the caller's to decide.
    ///
    /// # Errors
    ///
    /// The template's own error, which askama reports with the value or filter
    /// that failed. Rendering it again fails the same way.
    pub fn render(template: &impl Template) -> askama::Result<Self> {
        template.render().map(Self)
    }

    /// Markup the program wrote itself, such as an icon or a QR code. Nothing a
    /// person typed, and no name or tag from elsewhere, may reach it
    /// unescaped: it is spliced into pages as it is.
    pub fn trusted(markup: impl Into<String>) -> Self {
        Self(markup.into())
    }

    /// The markup as a Datastar patch, which morphs each top-level element into
    /// the page's element with the same id. The SDK's builder methods target
    /// something else: `.selector("#list").mode(ElementPatchMode::Append)`.
    ///
    /// Line breaks are normalised as an HTML parser normalises them, `\r\n`
    /// and a lone `\r` each becoming `\n`, so the page ends up with what a
    /// page load of the same markup would give. Left in, a `\r` would end its
    /// SSE line early, and the rest of that line would reach Datastar without
    /// the `elements` prefix that marks it as markup.
    #[must_use]
    pub fn patch(&self) -> PatchElements {
        PatchElements::new(lines(&self.0))
    }

    /// The markup as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `markup` with HTML's line breaks: CRLF and lone CRs as LF.
fn lines(markup: &str) -> Cow<'_, str> {
    if markup.contains('\r') {
        Cow::Owned(markup.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(markup)
    }
}

impl askama::filters::HtmlSafe for Html {}

impl fmt::Display for Html {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Deref for Html {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Html {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Fragments one after another, as a list's rows are.
impl FromIterator<Html> for Html {
    fn from_iter<I: IntoIterator<Item = Html>>(parts: I) -> Self {
        Self(parts.into_iter().map(|part| part.0).collect())
    }
}

/// A page, or a fragment for Datastar to morph in by its ids, as `text/html`.
impl IntoResponse for Html {
    fn into_response(self) -> Response {
        axum::response::Html(self.0).into_response()
    }
}

impl From<Html> for String {
    fn from(html: Html) -> Self {
        html.0
    }
}

#[cfg(test)]
mod tests {
    use super::lines;

    #[test]
    fn line_breaks_become_what_an_html_parser_makes_of_them() {
        assert_eq!(lines("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(lines("\r\r\n"), "\n\n");
    }

    #[test]
    fn markup_without_a_carriage_return_is_not_copied() {
        assert!(matches!(lines("a\nb"), std::borrow::Cow::Borrowed("a\nb")));
    }
}
