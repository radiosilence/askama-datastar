//! A page's Content Security Policy, as a value built from the one every
//! server starts with.
//!
//! The servers share one policy: everything from their own origin, nothing
//! inline, no frames, no plugins, no `<base>`. What differs between them is
//! small and named: Datastar's `'unsafe-eval'`, an import map's hash, no script
//! at all, an extra source such as `blob:`. A builder over the shared policy
//! keeps each server's difference visible as the one call that makes it, rather
//! than as a byte in a copied string.

use std::fmt;

use axum::http::HeaderValue;
use sha2::{Digest as _, Sha256};

/// A Content Security Policy, starting from the one every page gets:
///
/// ```text
/// default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;
/// font-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none';
/// base-uri 'none'; object-src 'none'
/// ```
///
/// `data:` images are there for the glyphs a stylesheet carries inline, such as
/// a checkbox's tick. Nothing allows inline script or style: that is the
/// policy's point, and no method here adds `'unsafe-inline'` for you.
/// `form-action` governs where a form's submission ends up, redirects
/// included: a POST answered with a redirect to another origin, such as a
/// sign-in handing back to an OAuth client, needs that origin allowed too.
///
/// ```
/// use askama_datastar::Csp;
///
/// let map = r#"{"imports":{"app/player":"/assets/player.js?v=0123"}}"#;
/// let csp = Csp::new().datastar().import_map(map).allow("img-src", "blob:");
/// assert!(csp.to_string().starts_with(
///     "default-src 'self'; script-src 'self' 'unsafe-eval' 'sha256-",
/// ));
///
/// // A sign-in page that runs nothing.
/// let none = Csp::new().deny("script-src");
/// assert!(none.to_string().contains("script-src 'none';"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Csp {
    /// Each directive and its sources, in the order the header lists them.
    directives: Vec<(&'static str, Vec<String>)>,
}

/// The policy [`Csp::new`] starts from, as STANDARD §8 gives it.
const BASE: [(&str, &[&str]); 10] = [
    ("default-src", &["'self'"]),
    ("script-src", &["'self'"]),
    ("style-src", &["'self'"]),
    ("img-src", &["'self'", "data:"]),
    ("font-src", &["'self'"]),
    ("connect-src", &["'self'"]),
    ("form-action", &["'self'"]),
    ("frame-ancestors", &["'none'"]),
    ("base-uri", &["'none'"]),
    ("object-src", &["'none'"]),
];

impl Csp {
    /// The policy every page starts from: everything from this origin,
    /// nothing inline, never framed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            directives: BASE
                .iter()
                .map(|(name, sources)| (*name, sources.iter().map(|s| (*s).to_owned()).collect()))
                .collect(),
        }
    }

    /// Allows what a page loading Datastar needs: its script from this origin,
    /// and `'unsafe-eval'`, since Datastar compiles its attribute expressions
    /// with `Function`.
    ///
    /// With eval allowed, escaping is what keeps text from running: anything
    /// that reaches a `data-*` attribute unescaped is code. Keep text out of
    /// Datastar expressions and in `data-*` attributes read with `el.dataset`.
    #[must_use]
    pub fn datastar(self) -> Self {
        self.allow("script-src", "'self'")
            .allow("script-src", "'unsafe-eval'")
    }

    /// Allows one inline import map, by the hash of its text: `map` is exactly
    /// what goes between `<script type="importmap">` and `</script>`, and any
    /// other byte there, a trailing newline included, blocks it.
    #[must_use]
    pub fn import_map(self, map: &str) -> Self {
        let hash = base64(&Sha256::digest(map.as_bytes()));
        self.allow("script-src", format!("'sha256-{hash}'"))
    }

    /// Adds `source` to `directive`, adding the directive if the policy lacks
    /// it: `allow("img-src", "blob:")` for previews of files about to be
    /// uploaded, `allow("form-action", "https://client.example")` for a form
    /// posting elsewhere or answered with a redirect there. A directive that
    /// was `'none'` becomes `source` alone.
    ///
    /// A directive the policy lacked no longer falls back to `default-src`, so
    /// `allow("media-src", "blob:")` refuses media from this origin unless
    /// `'self'` is allowed too.
    ///
    /// # Panics
    ///
    /// If `directive` is not lowercase letters and hyphens, or `source` is
    /// empty or holds whitespace, a control character, `;` or `,`, any of
    /// which would let it end the directive or the policy early; or if
    /// `source` is `'none'`, which is [`deny`](Self::deny)'s. Both are the
    /// program's own text, so a panic is a mistake caught where it is made.
    #[must_use]
    pub fn allow(mut self, directive: &'static str, source: impl Into<String>) -> Self {
        let source = source.into();
        assert_directive(directive);
        assert!(
            source != "'none'",
            "allow({directive:?}, \"'none'\") would be undone by any other source: use deny({directive:?})"
        );
        assert!(
            !source.is_empty()
                && source
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b';' && byte != b','),
            "{source:?} is not a CSP source: it must be one printable ASCII word with no `;` or `,`"
        );
        match self
            .directives
            .iter_mut()
            .find(|(name, _)| *name == directive)
        {
            Some((_, sources)) if sources.iter().any(|s| *s == "'none'") => *sources = vec![source],
            Some((_, sources)) => {
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
            None => self.directives.push((directive, vec![source])),
        }
        self
    }

    /// Sets `directive` to `'none'`, adding it if the policy lacks it:
    /// `deny("script-src")` for a page with no script, `deny("form-action")`
    /// for one with no form.
    ///
    /// # Panics
    ///
    /// If `directive` is not lowercase letters and hyphens.
    #[must_use]
    pub fn deny(mut self, directive: &'static str) -> Self {
        assert_directive(directive);
        let none = vec!["'none'".to_owned()];
        match self
            .directives
            .iter_mut()
            .find(|(name, _)| *name == directive)
        {
            Some((_, sources)) => *sources = none,
            None => self.directives.push((directive, none)),
        }
        self
    }

    /// The policy as a header's value, for a route that sets its own.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "it cannot panic: `allow` and `deny` admit only printable ASCII"
    )]
    pub fn header_value(&self) -> HeaderValue {
        HeaderValue::try_from(self.to_string())
            .expect("every directive and source was checked as it was added")
    }
}

impl Csp {
    /// Whether `frame-ancestors` is `'none'`, which `X-Frame-Options: DENY`
    /// repeats.
    pub(crate) fn denies_frames(&self) -> bool {
        self.directives
            .iter()
            .any(|(name, sources)| *name == "frame-ancestors" && sources == &["'none'"])
    }
}

impl Default for Csp {
    fn default() -> Self {
        Self::new()
    }
}

/// The policy as the header writes it: `name source source; name source`.
impl fmt::Display for Csp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, sources)) in self.directives.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            f.write_str(name)?;
            for source in sources {
                write!(f, " {source}")?;
            }
        }
        Ok(())
    }
}

fn assert_directive(directive: &str) {
    assert!(
        !directive.is_empty()
            && directive
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-'),
        "{directive:?} is not a CSP directive name"
    );
}

/// `bytes` in standard, padded base64, which is what a CSP hash source takes.
/// Written here, since a hash is the one thing in the crate encoded so.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk
            .iter()
            .zip([16, 8, 0])
            .fold(0_u32, |group, (byte, shift)| {
                group | u32::from(*byte) << shift
            });
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(group >> shift & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_is_the_standard_padded_alphabet() {
        // RFC 4648's test vectors.
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(text.as_bytes()), encoded, "{text:?}");
        }
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=", "the last two symbols");
    }
}
