//! The escaping guard finds text escaped other than once.

use std::time::Duration;

use askama::Template;
use askama_datastar::escaping::{Guard, Problem, escaped, get};
use askama_datastar::{Html, events};
use axum::Router;
use axum::extract::Query;
use axum::response::{IntoResponse, Response};

const HOSTILE: &str = r#"<em>hostile "q" & 'x'</em>"#;

fn templates() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("askama-datastar-escaping-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("cards")).unwrap();
    std::fs::write(dir.join("page.html"), "<main>{{ card }}</main>").unwrap();
    std::fs::write(
        dir.join("cards").join("card.html"),
        r#"<article class="card"><h2>{{ title }}</h2></article>"#,
    )
    .unwrap();
    dir
}

fn guard() -> Guard {
    Guard::new(&[HOSTILE]).templates(templates()).unwrap()
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<article class="card"><h2>{{ title }}</h2></article>"#
)]
struct Card<'a> {
    title: &'a str,
}

/// The card spliced in as markup, as it should be.
#[derive(Template)]
#[template(ext = "html", source = "<main>{{ card }}</main>")]
struct Page {
    card: Html,
}

/// The card spliced in as text, the mistake the guard is for.
#[derive(Template)]
#[template(ext = "html", source = "<main>{{ card }}</main>")]
struct Spliced {
    card: String,
}

#[test]
fn an_answer_with_the_text_escaped_once_has_no_problems() {
    let card = Html::render(&Card { title: HOSTILE }).unwrap();
    let page = Page { card }.render().unwrap();
    assert_eq!(guard().problems(&page), []);
    assert!(page.contains(&escaped(HOSTILE)), "{page}");
}

#[test]
fn a_fragment_spliced_in_as_text_is_escaped_markup() {
    let card = Card { title: HOSTILE }.render().unwrap();
    let page = Spliced { card }.render().unwrap();
    let problems = guard().problems(&page);
    assert!(
        problems
            .iter()
            .any(|p| matches!(p, Problem::EscapedMarkup(at) if at.starts_with("&#60;article"))),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| matches!(p, Problem::EscapedTwice(_))),
        "the title inside it was escaped twice: {problems:?}"
    );
}

#[test]
fn text_left_as_markup_is_found_outside_signals() {
    let page = format!("<main><h2>{HOSTILE}</h2></main>");
    assert!(matches!(
        guard().problems(&page)[..],
        [Problem::Unescaped(_)]
    ));

    let signals =
        format!("event: datastar-patch-signals\ndata: signals {{\"q\":\"{HOSTILE}\"}}\n\n");
    assert_eq!(
        guard().problems(&signals),
        [],
        "signals are JSON, not markup"
    );
}

#[test]
fn elements_the_templates_do_not_write_can_be_added() {
    let page = "<main>&lt;dialog open&gt;</main>";
    assert_eq!(guard().problems(page), []);
    assert_eq!(guard().elements(["dialog"]).problems(page).len(), 1);
    assert_eq!(
        guard()
            .markup("<dialog open></dialog>")
            .problems(page)
            .len(),
        1
    );
    let em = "<main>&lt;em&gt;</main>";
    assert_eq!(
        guard().markup("<em>").problems(em),
        [],
        "the hostile text's own elements come out escaped by right"
    );
}

#[test]
#[should_panic(expected = "does not start with an element")]
fn hostile_text_starts_with_markup() {
    let _ = Guard::new(&["hostile <em>"]);
}

#[derive(serde::Deserialize)]
struct Q {
    title: String,
}

async fn card(Query(q): Query<Q>, headers: axum::http::HeaderMap) -> Response {
    let card = Html::render(&Card { title: &q.title }).unwrap();
    if askama_datastar::is_datastar(&headers) {
        events([card.patch().into()])
    } else {
        Page { card }
            .render()
            .map(axum::response::Html)
            .unwrap()
            .into_response()
    }
}

#[tokio::test]
async fn every_answer_is_read_through_the_router() {
    let router = Router::new().route("/card", axum::routing::get(card));
    let address = format!(
        "/card?title={}",
        form_urlencoded::byte_serialize(HOSTILE.as_bytes()).collect::<String>()
    );
    let guard = guard();
    for in_place in [false, true] {
        let (status, answer) = get(&router, &address, in_place, Duration::from_secs(1)).await;
        assert!(status.is_success());
        assert_eq!(guard.problems(&answer), [], "{answer}");
        assert!(answer.contains(&escaped(HOSTILE)), "{answer}");
        assert_eq!(answer.starts_with("event:"), in_place, "{answer}");
    }
}
