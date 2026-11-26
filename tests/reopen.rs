//! The attributes that keep a live stream open: the URL stays text, and every
//! expression is the same whatever the URL.

use askama_datastar::reopen;

/// The attributes after `data-reopen-url`'s value.
fn after_url(attributes: &str) -> String {
    let rest = attributes
        .strip_prefix(" data-reopen-url=\"")
        .expect("data-reopen-url first");
    rest[rest.find('"').expect("data-reopen-url closes")..].to_owned()
}

#[test]
fn the_url_is_an_escaped_attribute_value() {
    let attributes = reopen(r#"/stream?a=1&b="2"&c='<3>'"#);
    assert!(
        attributes.starts_with(
            r#" data-reopen-url="/stream?a=1&amp;b=&quot;2&quot;&amp;c=&#39;&lt;3&gt;&#39;" "#
        ),
        "{attributes}"
    );
}

#[test]
fn no_expression_holds_the_url() {
    let plain = reopen("/stream");
    let hostile = reopen(r#"/stream"); alert(1); ("#);
    assert_eq!(
        after_url(&plain),
        after_url(&hostile),
        "an expression changed with the URL"
    );
    assert!(
        !after_url(&plain).contains("/stream"),
        "the URL reached an expression"
    );
}

#[test]
fn only_a_path_on_this_origin_is_a_stream() {
    for url in [
        "https://elsewhere.example/stream",
        "//elsewhere.example/stream",
        "/\\elsewhere.example/stream",
        "stream",
        "javascript:alert(1)",
        "",
    ] {
        assert!(
            std::panic::catch_unwind(|| reopen(url)).is_err(),
            "{url:?} was taken for a stream"
        );
    }
}

#[test]
fn the_stream_is_followed_before_it_is_opened() {
    let attributes = reopen("/stream");
    let at = |attribute: &str| {
        attributes
            .find(attribute)
            .unwrap_or_else(|| panic!("no {attribute}"))
    };
    // Datastar applies attributes in order, and a request announces its start
    // at once: heard by nothing, the first stream would count as ended.
    assert!(
        at(" data-on:datastar-fetch=\"") < at(" data-init__delay."),
        "the stream opens before anything listens for it"
    );
    at(" data-on-interval__duration.5s=\"");
    at(" data-on:visibilitychange__document=\"");
}

#[test]
fn the_state_cannot_collide_with_the_hosts_own() {
    let attributes = after_url(&reopen("/stream"));
    for property in ["el.open", "el.seen", "el.refused", "el.dataset.url"] {
        assert!(
            !attributes.contains(property),
            "{property} would clash with a <dialog> or <details> host, or the page's own data-url"
        );
    }
}

#[test]
fn only_401_and_403_stop_the_reopening() {
    assert!(
        reopen("/stream").contains("['401', '403'].includes(evt.detail.argsRaw.status)"),
        "a temporary status such as 429 would stop the stream for good"
    );
}
