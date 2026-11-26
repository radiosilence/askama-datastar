//! The attributes that keep a live stream open: the URL stays text, and every
//! expression is the same whatever the URL.

use askama_datastar::reopen;

#[test]
fn the_url_is_an_escaped_attribute_value() {
    let attributes = reopen(r#"/stream?a=1&b="2"&c='<3>'"#);
    assert!(
        attributes.starts_with(
            r#" data-url="/stream?a=1&amp;b=&quot;2&quot;&amp;c=&#39;&lt;3&gt;&#39;" "#
        ),
        "{attributes}"
    );
}

#[test]
fn no_expression_holds_the_url() {
    let plain = reopen("/stream");
    let hostile = reopen(r#"/stream"); alert(1); ("#);
    let after_url = |attributes: &str| {
        let rest = attributes
            .strip_prefix(" data-url=\"")
            .expect("data-url first");
        rest[rest.find('"').expect("data-url closes")..].to_owned()
    };
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
        at(" data-on:datastar-fetch=\"") < at(r#" data-init="@get(el.dataset.url)""#),
        "the stream opens before anything listens for it"
    );
    at(" data-on-interval__duration.5s=\"");
    at(" data-on:visibilitychange__document=\"");
}
