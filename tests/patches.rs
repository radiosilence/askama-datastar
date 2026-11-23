//! Patches on the wire, against Datastar's SSE format: an `event:` line naming
//! the event, then one `data:` line per field (`selector`, `mode`, `signals`,
//! one `elements` per line of markup), then a blank line.

use askama::Template;
use askama_datastar::{ElementPatchMode, Html, PatchSignals, events};
use axum::response::Response;

async fn body(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("an events answer is a finite body");
    String::from_utf8(bytes.to_vec()).expect("events are UTF-8")
}

#[derive(Template)]
#[template(
    ext = "html",
    source = "<ul id=\"rows\">\n{% for row in rows %}  <li>{{ row }}</li>\n{% endfor %}</ul>"
)]
struct Rows<'a> {
    rows: &'a [&'a str],
}

#[tokio::test]
async fn a_rendered_template_is_one_elements_line_per_line_of_markup() {
    let rows = Html::render(&Rows {
        rows: &["moss", "<lichen>"],
    })
    .expect("renders");
    assert_eq!(
        body(events([rows.patch()])).await,
        concat!(
            "event: datastar-patch-elements\n",
            "data: elements <ul id=\"rows\">\n",
            "data: elements   <li>moss</li>\n",
            "data: elements   <li>&#60;lichen&#62;</li>\n",
            "data: elements </ul>\n",
            "\n",
        ),
    );
}

#[tokio::test]
async fn a_selector_and_mode_come_before_the_elements() {
    let row = Html::trusted("<li>fern</li>")
        .patch()
        .selector("#rows")
        .mode(ElementPatchMode::Append);
    assert_eq!(
        body(events([row])).await,
        concat!(
            "event: datastar-patch-elements\n",
            "data: selector #rows\n",
            "data: mode append\n",
            "data: elements <li>fern</li>\n",
            "\n",
        ),
    );
}

#[tokio::test]
async fn the_default_mode_is_left_for_datastar_to_assume() {
    let patch = Html::trusted("<p id=\"a\"></p>")
        .patch()
        .mode(ElementPatchMode::Outer);
    assert_eq!(
        body(events([patch])).await,
        "event: datastar-patch-elements\ndata: elements <p id=\"a\"></p>\n\n",
    );
}

#[tokio::test]
async fn a_carriage_return_cannot_end_an_elements_line_early() {
    // Left in, the `\r` would end the SSE line, and `<b>two</b>` would arrive
    // as a line of its own without the `elements` prefix.
    let patch = Html::trusted("<p id=\"a\">one\r<b>two</b>\r\nthree</p>").patch();
    assert_eq!(
        body(events([patch])).await,
        concat!(
            "event: datastar-patch-elements\n",
            "data: elements <p id=\"a\">one\n",
            "data: elements <b>two</b>\n",
            "data: elements three</p>\n",
            "\n",
        ),
    );
}

#[tokio::test]
async fn signals_are_sent_as_the_json_given() {
    let signals = PatchSignals::new(r#"{"q":"","active":0,"_open":false}"#);
    assert_eq!(
        body(events([signals])).await,
        "event: datastar-patch-signals\ndata: signals {\"q\":\"\",\"active\":0,\"_open\":false}\n\n",
    );
}

#[tokio::test]
async fn multi_line_signals_keep_the_prefix_on_every_line() {
    let signals = PatchSignals::new("{\n  \"q\": \"moss\"\n}").only_if_missing(true);
    assert_eq!(
        body(events([signals])).await,
        concat!(
            "event: datastar-patch-signals\n",
            "data: onlyIfMissing true\n",
            "data: signals {\n",
            "data: signals   \"q\": \"moss\"\n",
            "data: signals }\n",
            "\n",
        ),
    );
}

#[tokio::test]
async fn events_go_in_the_order_given() {
    let answer = events([
        Html::trusted("<p id=\"flash\">Saved.</p>").patch().into(),
        PatchSignals::new(r#"{"name":""}"#).into(),
    ]);
    assert_eq!(
        body(answer).await,
        concat!(
            "event: datastar-patch-elements\n",
            "data: elements <p id=\"flash\">Saved.</p>\n",
            "\n",
            "event: datastar-patch-signals\n",
            "data: signals {\"name\":\"\"}\n",
            "\n",
        ),
    );
}

#[tokio::test]
async fn an_element_can_be_removed_by_selector() {
    let remove = askama_datastar::PatchElements::new_remove("#row-7");
    assert_eq!(
        body(events([remove])).await,
        "event: datastar-patch-elements\ndata: selector #row-7\ndata: mode remove\n\n",
    );
}
