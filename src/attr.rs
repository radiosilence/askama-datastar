//! What every attribute this crate writes shares: how a value is escaped,
//! which URLs it may name, and which signal names an expression may read.

/// `text` escaped as an attribute value, with the numeric references askama's
/// HTML escaper writes, so a value reads the same whichever wrote it.
pub(crate) fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&#38;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&#60;"),
            '>' => out.push_str("&#62;"),
            c => out.push(c),
        }
    }
}

/// Panics unless `path` is a path on the page's own origin. It must start
/// with `/`, and not with `//` or `/\`, which a browser reads as another host.
/// It may hold no control or space character either: a browser strips tabs
/// and line breaks from a URL, so `/\t/host` would become `//host`.
pub(crate) fn assert_on_origin(path: &str) {
    assert!(
        path.starts_with('/')
            && !path.starts_with("//")
            && !path.starts_with("/\\")
            && !path.chars().any(|c| c.is_control() || c.is_whitespace()),
        "{path:?} is not a path on this origin"
    );
}

/// Panics unless `name` is a signal name: letters, digits, `_` and `.`. It is
/// written into an expression, so it is the program's own text.
pub(crate) fn assert_signal_name(name: &str) {
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')),
        "{name:?} is not a signal name"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn values_are_escaped_as_askama_escapes_them() {
        let text = r#"<a "&'>"#;
        let mut out = String::new();
        super::escape(text, &mut out);
        let askama = askama::filters::escape(text, askama::filters::Html)
            .unwrap()
            .to_string();
        assert_eq!(out, askama);
    }

    #[test]
    #[should_panic(expected = "is not a path on this origin")]
    fn a_path_a_browser_strips_into_another_host_is_refused() {
        super::assert_on_origin("/\t/evil.example/");
    }
}
