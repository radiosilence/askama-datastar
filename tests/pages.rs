//! A page drawn whole for a browser and in place for Datastar, through a
//! router.

#![allow(clippy::unused_async_trait_impl)]

use askama_datastar::Html;
use askama_datastar::page::{self, Body, Layout, Page, View, Whole};
use axum::Router;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower::ServiceExt as _;

#[derive(Clone)]
struct App;

/// A layout with a title and a main region, and signals of its own.
struct Frame;

impl Layout for Frame {
    type State = App;
    type Place = &'static str;
    type Error = &'static str;

    async fn whole(_: &App, place: &&'static str, mut page: Whole) -> Result<Html, Self::Error> {
        page.signals.entry("rows").or_insert(100.into());
        Ok(Html::trusted(format!(
            "<title id=\"title\">{}</title><body{}><div hidden{}></div><main id=\"main\">{}</main></body>",
            page.title.as_deref().unwrap_or(place),
            page.attributes().as_str(),
            askama_datastar::reopen_on("/stream/search", &[Self::ADDRESS, "rows"]).as_str(),
            page.body.as_str(),
        )))
    }

    async fn in_place(_: &App, place: &&'static str, title: Option<&str>) -> Vec<Event> {
        vec![
            Html::trusted(format!(
                "<title id=\"title\">{}</title>",
                title.unwrap_or(place)
            ))
            .patch()
            .into(),
        ]
    }

    fn failed(_: &App, error: &'static str, in_place: bool) -> Response {
        if in_place {
            askama_datastar::events([Html::trusted(format!("<p id=\"flash\">{error}</p>"))
                .patch()
                .into()])
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, error).into_response()
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Query {
    q: String,
    fail: bool,
    moved: bool,
}

struct Search;

impl Page for Search {
    type Layout = Frame;
    const PLACE: &'static str = "search";
    type Args = View<Query>;

    async fn body(_: &App, View(query): View<Query>) -> Result<Body, &'static str> {
        if query.fail {
            return Err("the index is down");
        }
        if query.moved {
            return Ok(Body::redirect("/found?q=1"));
        }
        Ok(Body::new(Html::trusted("<p>results</p>"))
            .title(query.q)
            .signal("open", false)
            .status(StatusCode::ACCEPTED))
    }
}

fn router() -> Router {
    Router::new()
        .route("/search", get(page::serve::<Search>))
        .with_state(App)
}

async fn answer(uri: &str, datastar: bool) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut request = Request::get(uri);
    if datastar {
        request = request.header("datastar-request", "true");
    }
    let response = router()
        .oneshot(request.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn a_browser_gets_the_layout_around_the_body() {
    let (status, headers, page) = answer("/search?q=moss", false).await;
    assert_eq!(status, StatusCode::ACCEPTED, "the body's status: {page}");
    assert_eq!(headers["vary"], "datastar-request");
    assert!(page.contains("<title id=\"title\">moss</title>"), "{page}");
    assert!(
        page.contains("<main id=\"main\"><p>results</p></main>"),
        "{page}"
    );
    assert!(
        page.contains(r#"data-signals="{&quot;open&quot;:false,&quot;rows&quot;:100,&quot;url&quot;:&quot;/search?q=moss&quot;}""#),
        "the page's signals, the layout's and the address: {page}"
    );
    assert!(page.contains("history.pushState(null, '', $url)"), "{page}");
    assert!(
        page.contains(r#"data-reopen-url="/stream/search""#),
        "{page}"
    );
    assert!(page.contains(r#"data-effect="$url; $rows; "#), "{page}");
}

#[tokio::test]
async fn the_page_itself_gets_the_body_in_place_with_its_address() {
    let (status, headers, events) =
        answer("/search?q=moss&datastar=%7B%22url%22%3A%22%2F%22%7D", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/event-stream");
    let expected = concat!(
        "event: datastar-patch-elements\n",
        "data: selector #main\n",
        "data: mode inner\n",
        "data: elements <p>results</p>\n\n",
        "event: datastar-patch-elements\n",
        "data: elements <title id=\"title\">moss</title>\n\n",
        "event: datastar-patch-signals\n",
        "data: signals {\"open\":false,\"url\":\"/search?q=moss\"}\n\n",
    );
    assert_eq!(events, expected);
}

#[tokio::test]
async fn an_address_that_does_not_parse_is_the_default_view() {
    let (status, _, page) = answer("/search?fail=maybe", false).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{page}");
    assert!(
        page.contains("<title id=\"title\">search</title>"),
        "{page}"
    );
}

#[tokio::test]
async fn a_moved_page_redirects_a_browser_and_sends_the_page_there() {
    let (status, headers, _) = answer("/search?moved=true", false).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers["location"], "/found?q=1");

    let (_, _, events) = answer("/search?moved=true", true).await;
    assert_eq!(
        events,
        concat!(
            "event: datastar-patch-elements\n",
            "data: selector body\n",
            "data: mode append\n",
            "data: elements <div hidden data-go=\"/found?q=1\" data-init=\"location.assign(el.dataset.go)\"></div>\n\n",
        )
    );
}

#[tokio::test]
async fn a_failure_is_the_layouts_to_answer() {
    let (status, _, text) = answer("/search?fail=true", false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, "the index is down");

    let (status, _, events) = answer("/search?fail=true", true).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        events.contains("<p id=\"flash\">the index is down</p>"),
        "{events}"
    );
}

#[tokio::test]
async fn an_action_answers_with_the_page_it_leads_to() {
    let body = Body::new(Html::trusted("<p>found</p>")).title("found".to_owned());
    let events = page::shown::<Search>(&App, body, "/search?q=found").await;
    assert_eq!(events.len(), 3, "the body, the title and the signals");
}

#[test]
#[should_panic(expected = "is not a path on this origin")]
fn a_page_goes_only_to_a_path_on_its_own_origin() {
    let _ = page::go("//evil.example/");
}
