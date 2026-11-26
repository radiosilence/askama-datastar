//! A notes board: pages with addresses, loaded whole or in place, kept current
//! by a live stream. A note added in one tab shows in every other.
//!
//! `cargo run --example notes`, then open <http://127.0.0.1:3000>.

use std::sync::{Arc, Mutex, PoisonError};

use askama::Template;
use askama_datastar::{
    ElementPatchMode, Html, Last, Pace, PatchSignals, Signals, View, address, clear, events, go,
    history, is_datastar, paced, reopen_on, require_datastar_on_post, script, stream,
};
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use futures_util::StreamExt as _;
use tokio::sync::watch;

/// The notes, by id (a deleted one leaves its id behind), and a counter bumped
/// on every change.
#[derive(Clone, Default)]
pub(crate) struct Notes {
    all: Arc<Mutex<Vec<Option<String>>>>,
    changed: Arc<watch::Sender<u64>>,
}

impl Notes {
    pub(crate) fn add(&self, text: &str) -> usize {
        let mut all = self.all.lock().unwrap_or_else(PoisonError::into_inner);
        all.push(Some(text.to_owned()));
        self.changed.send_modify(|n| *n += 1);
        all.len() - 1
    }

    fn delete(&self, id: usize) {
        let mut all = self.all.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(note) = all.get_mut(id) {
            *note = None;
        }
        self.changed.send_modify(|n| *n += 1);
    }

    fn get(&self, id: usize) -> Option<String> {
        let all = self.all.lock().unwrap_or_else(PoisonError::into_inner);
        all.get(id).cloned().flatten()
    }

    /// The notes holding `q`, newest first.
    fn matching(&self, q: &str) -> Vec<(usize, String)> {
        let all = self.all.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found: Vec<_> = all
            .iter()
            .enumerate()
            .filter_map(|(id, note)| Some((id, note.clone()?)))
            .filter(|(_, note)| note.contains(q))
            .collect();
        found.reverse();
        found
    }
}

fn render(template: &impl Template) -> Html {
    Html::render(template).unwrap_or_default()
}

// The document, and a page shown whole or in place.

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
{{ title }}
<script type="module" src="{{ script }}"></script>
</head>
<body data-signals="{{ signals }}"{{ history }}>
<div hidden{{ live }}></div>
<nav><a href="/" data-on:click__prevent="@get(el.href)">notes</a> {{ count }}</nav>
<main id="main">{{ body }}</main>
</body>
</html>"#
)]
struct Document {
    title: Html,
    script: String,
    signals: String,
    history: Html,
    live: Html,
    count: Html,
    body: Html,
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<title id="title">{{ name }} · notes</title>"#
)]
struct Title<'a> {
    name: &'a str,
}

#[derive(Template)]
#[template(ext = "html", source = r#"<span id="count">{{ n }} notes</span>"#)]
struct Count {
    n: usize,
}

/// The page at `uri`: the whole document for a browser, or, for the page
/// itself following a link, the body morphed into `main`, the title, and the
/// address, which the address bar follows.
fn show(notes: &Notes, headers: &HeaderMap, uri: &Uri, title: &str, body: Html) -> Response {
    let url = address(uri);
    if is_datastar(headers) {
        return events(in_place(&url, title, &body));
    }
    let document = Document {
        title: render(&Title { name: title }),
        script: script::url("/datastar.js"),
        signals: serde_json::json!({ "url": url, "text": "" }).to_string(),
        history: history("url"),
        live: reopen_on("/live", &["url"]),
        count: render(&Count {
            n: notes.matching("").len(),
        }),
        body,
    };
    render(&document).into_response()
}

fn in_place(url: &str, title: &str, body: &Html) -> Vec<Event> {
    vec![
        body.patch()
            .selector("#main")
            .mode(ElementPatchMode::Inner)
            .into(),
        render(&Title { name: title }).patch().into(),
        PatchSignals::new(serde_json::json!({ "url": url }).to_string()).into(),
    ]
}

// The pages.

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Search {
    q: String,
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<ul id="notes">
{%- for (id, text) in notes %}
<li><a href="/notes/{{ id }}" data-on:click__prevent="@get(el.href)">{{ text }}</a></li>
{%- else %}
<li>No notes.</li>
{%- endfor %}
</ul>"#
)]
struct List {
    notes: Vec<(usize, String)>,
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<form data-on:submit__prevent="@get('/', {contentType: 'form'})">
<input name="q" value="{{ q }}" placeholder="Search"> <button>Search</button>
</form>
{{ list }}
<p><input data-bind:text placeholder="A new note"> <button data-on:click="@post('/notes')">Add</button></p>
<p id="error"></p>"#
)]
struct Index {
    q: String,
    list: Html,
}

impl Index {
    fn of(notes: &Notes, q: String) -> Html {
        let list = render(&List {
            notes: notes.matching(&q),
        });
        render(&Self { q, list })
    }
}

async fn index(
    State(notes): State<Notes>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    View(search): View<Search>,
) -> Response {
    show(&notes, &headers, &uri, "notes", Index::of(&notes, search.q))
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<p>{{ text }}</p>
<button data-action="/notes/{{ id }}/delete" data-on:click="@post(el.dataset.action)">Delete</button>"#
)]
struct Note<'a> {
    id: usize,
    text: &'a str,
}

async fn note(
    State(notes): State<Notes>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<usize>,
) -> Response {
    if let Some(text) = notes.get(id) {
        return show(
            &notes,
            &headers,
            &uri,
            &text,
            render(&Note { id, text: &text }),
        );
    }
    let missing = Html::trusted("<p>There is no such note.</p>");
    let page = show(&notes, &headers, &uri, "not found", missing);
    // Datastar applies an answer's events only when its status is 2xx.
    if is_datastar(&headers) {
        page
    } else {
        (StatusCode::NOT_FOUND, page).into_response()
    }
}

/// The newest note's page, wherever that is.
async fn latest(State(notes): State<Notes>, headers: HeaderMap) -> Response {
    let to = notes
        .matching("")
        .first()
        .map_or_else(|| "/".to_owned(), |(id, _)| format!("/notes/{id}"));
    if is_datastar(&headers) {
        events([go(&to).into()])
    } else {
        Redirect::to(&to).into_response()
    }
}

// The actions.

#[derive(serde::Deserialize)]
struct NewNote {
    text: String,
}

/// Adds a note. The live stream draws it, so the answer only clears the form,
/// or says what is wrong.
async fn add(State(notes): State<Notes>, Json(new): Json<NewNote>) -> Response {
    if new.text.trim().is_empty() {
        let error = Html::trusted(r#"<p id="error">Write something first.</p>"#);
        return events([error.patch().into()]);
    }
    notes.add(new.text.trim());
    events([
        clear("#error").into(),
        PatchSignals::new(r#"{"text":""}"#).into(),
    ])
}

/// Deletes a note, and shows the list in its place.
async fn delete(State(notes): State<Notes>, Path(id): Path<usize>) -> Response {
    notes.delete(id);
    events(in_place("/", "notes", &Index::of(&notes, String::new())))
}

// The live stream.

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Shown {
    url: String,
}

/// Every page's stream: the count, and the list on the page that shows it,
/// as its address's search finds it.
async fn live(State(notes): State<Notes>, Signals(shown): Signals<Shown>) -> Response {
    let changes = futures_util::stream::unfold(
        (notes.changed.subscribe(), true),
        |(mut changed, first)| async move {
            if !first {
                changed.changed().await.ok()?;
            }
            changed.borrow_and_update();
            Some(((), (changed, false)))
        },
    );
    let on_index = shown.url.split('?').next() == Some("/");
    let View(search) = View::<Search>::of(&shown.url);
    let (mut count, mut list) = (Last::new(), Last::new());
    let redraws = paced(changes, Pace::default()).flat_map(move |()| {
        let mut events = Vec::new();
        events.extend(count.send(&render(&Count {
            n: notes.matching("").len(),
        })));
        if on_index {
            events.extend(list.send(&render(&List {
                notes: notes.matching(&search.q),
            })));
        }
        futures_util::stream::iter(events)
    });
    stream(redraws)
}

pub(crate) fn routes(notes: Notes) -> Router {
    Router::new()
        .route("/notes", post(add))
        .route("/notes/{id}/delete", post(delete))
        .layer(middleware::from_fn(require_datastar_on_post))
        .route("/", get(index))
        .route("/notes/{id}", get(note))
        .route("/latest", get(latest))
        .route("/live", get(live))
        .route("/datastar.js", get(script::serve))
        .with_state(notes)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, routes(Notes::default())).await
}
