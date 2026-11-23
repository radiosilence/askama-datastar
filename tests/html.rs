//! A fragment written once: the markup a page embeds is the markup a patch
//! sends, whether the fragment is a template of its own or a block of the
//! page's.

use askama::Template;
use askama_datastar::{Html, events};

#[derive(Template)]
#[template(ext = "html", source = "<p id=\"status\">{{ text }}</p>")]
struct Status<'a> {
    text: &'a str,
}

#[derive(Template)]
#[template(ext = "html", source = "<main>{{ status }}<p>{{ note }}</p></main>")]
struct Page<'a> {
    status: Html,
    note: &'a str,
}

async fn sent(html: &Html) -> String {
    let bytes = axum::body::to_bytes(events([html.patch()]).into_body(), usize::MAX)
        .await
        .expect("a finished body");
    let body = String::from_utf8(bytes.to_vec()).expect("UTF-8");
    body.lines()
        .filter_map(|line| line.strip_prefix("data: elements "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_page_and_a_patch_carry_the_same_markup_for_a_fragment() {
    let status = Html::render(&Status { text: "<scanning>" }).expect("renders");
    let page = Page {
        status: status.clone(),
        note: "<note>",
    }
    .render()
    .expect("renders");
    assert_eq!(
        page, "<main><p id=\"status\">&#60;scanning&#62;</p><p>&#60;note&#62;</p></main>",
        "the fragment is embedded as it is, and the page's own text escaped"
    );
    assert!(page.contains(&sent(&status).await));
}

#[derive(Template)]
#[template(
    ext = "html",
    source = "<main>{% block results %}<ul id=\"results\">{% for r in results %}<li>{{ r }}</li>{% endfor %}</ul>{% endblock %}<footer>{{ footer }}</footer></main>",
    blocks = ["results"]
)]
struct Search<'a> {
    results: &'a [&'a str],
    footer: &'a str,
}

#[tokio::test]
async fn a_block_of_the_page_patches_the_markup_the_page_holds() {
    let search = Search {
        results: &["moss", "fern"],
        footer: "two found",
    };
    let page = search.render().expect("renders");
    let block = Html::render(&search.as_results()).expect("renders");
    assert_eq!(
        sent(&block).await,
        "<ul id=\"results\"><li>moss</li><li>fern</li></ul>"
    );
    assert!(page.contains(block.as_str()));
}

#[test]
fn rows_collect_into_one_fragment() {
    let rows: Html = ["moss", "fern"]
        .into_iter()
        .map(|text| Html::render(&Status { text }).expect("renders"))
        .collect();
    assert_eq!(
        rows.as_str(),
        "<p id=\"status\">moss</p><p id=\"status\">fern</p>"
    );
}

#[test]
fn markup_answered_on_its_own_is_html() {
    let response = axum::response::IntoResponse::into_response(Html::trusted("<p id=\"a\"></p>"));
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
}

#[test]
fn no_markup_can_be_held_in_a_static() {
    static NOTHING: Html = Html::new();
    assert_eq!(NOTHING.as_str(), "");
    assert_eq!(NOTHING, Html::default());
}
