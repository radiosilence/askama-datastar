//! A notes board: a routed application built from `page`, `live` and the
//! other pieces. Each page has an address, loads whole or in place, and is kept
//! current by one live stream; a note added in one tab shows in every other.
//!
//! `cargo run --example notes`, then open <http://127.0.0.1:3000>.

// Most of these trait methods have nothing to wait on, which the traits allow.
#![allow(clippy::unused_async_trait_impl)]

use std::sync::{Arc, Mutex};

use askama::Template;
use askama_datastar::live::{self, Changes, Draw, Last, Timing};
use askama_datastar::page::{self, Body, Layout, Page, View, Whole};
use askama_datastar::{
    Html, PatchSignals, Signals, clear, events, reopen_on, require_datastar_on_post, script, stream,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use tokio::sync::watch;

/// The application's state: the notes, and a counter bumped on every change.
#[derive(Clone, Default)]
pub(crate) struct Notes {
    /// Each note by its id; a deleted note leaves its id behind.
    all: Arc<Mutex<Vec<Option<String>>>>,
    changed: Arc<watch::Sender<u64>>,
}

impl Notes {
    pub(crate) fn add(&self, text: &str) -> usize {
        let mut all = self
            .all
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        all.push(Some(text.to_owned()));
        self.changed.send_modify(|n| *n += 1);
        all.len() - 1
    }

    fn delete(&self, id: usize) {
        if let Some(note) = self
            .all
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(id)
        {
            *note = None;
        }
        self.changed.send_modify(|n| *n += 1);
    }

    fn get(&self, id: usize) -> Option<String> {
        self.all
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
            .flatten()
    }

    /// The notes holding `q`, newest first.
    fn matching(&self, q: &str) -> Vec<(usize, String)> {
        let all = self
            .all
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut found: Vec<(usize, String)> = all
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

// The document.

/// The menu's entries, and the one a page is shown under.
const MENU: [(&str, &str); 3] = [("/", "notes"), ("/latest", "latest"), ("/about", "about")];

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
<body{{ attributes }}>
<div hidden{{ live }}></div>
<nav>
{%- for (href, label) in menu %}
<a href="{{ href }}" data-on:click__prevent="@get(el.href)"{% if self.is_current(label) %} aria-current="page"{% endif %}>{{ label }}</a>
{%- endfor %}
{{ count }}
</nav>
<output id="flash"></output>
<main id="main">{{ body }}</main>
</body>
</html>"#
)]
struct Document<'a> {
    title: Html,
    script: String,
    attributes: Html,
    live: Html,
    menu: [(&'a str, &'a str); 3],
    current: &'a str,
    count: Html,
    body: Html,
}

impl Document<'_> {
    fn is_current(&self, label: &str) -> bool {
        label == self.current
    }
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

impl Count {
    fn of(notes: &Notes) -> Self {
        Self {
            n: notes.matching("").len(),
        }
    }
}

/// The document every page is drawn in. A page's place is its menu entry.
struct Board;

impl Layout for Board {
    type State = Notes;
    type Place = &'static str;
    type Error = askama::Error;

    async fn whole(notes: &Notes, entry: &&'static str, mut page: Whole) -> askama::Result<Html> {
        page.signals.entry("text").or_insert("".into());
        Html::render(&Document {
            title: render(&Title {
                name: page.title.as_deref().unwrap_or(entry),
            }),
            script: script::url("/datastar.js"),
            attributes: page.attributes(),
            live: reopen_on("/live", &[Self::ADDRESS]),
            menu: MENU,
            current: entry,
            count: render(&Count::of(notes)),
            body: page.body,
        })
    }

    async fn in_place(_: &Notes, entry: &&'static str, title: Option<&str>) -> Vec<Event> {
        let title = render(&Title {
            name: title.unwrap_or(entry),
        });
        vec![title.patch().into(), clear("#flash").into()]
    }

    fn failed(_: &Notes, error: askama::Error, in_place: bool) -> Response {
        if in_place {
            let flash =
                Html::trusted(r#"<output id="flash">The page could not be drawn.</output>"#);
            events([flash.patch().into()])
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    }
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
struct IndexBody {
    q: String,
    list: Html,
}

/// Every note, or those a search finds.
struct Index;

impl Page for Index {
    type Layout = Board;
    const PLACE: &'static str = "notes";
    type Args = View<Search>;

    async fn body(notes: &Notes, View(search): View<Search>) -> askama::Result<Body> {
        let list = Html::render(&List {
            notes: notes.matching(&search.q),
        })?;
        let body = Html::render(&IndexBody { q: search.q, list })?;
        Ok(Body::new(body))
    }
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"{% if let Some(text) = text -%}
<p>{{ text }}</p>
<button data-action="/notes/{{ id }}/delete" data-on:click="@post(el.dataset.action)">Delete</button>
{%- else -%}
<p>There is no note {{ id }}.</p>
{%- endif %}"#
)]
struct NoteBody {
    id: usize,
    text: Option<String>,
}

/// One note.
struct Note;

impl Page for Note {
    type Layout = Board;
    const PLACE: &'static str = "notes";
    type Args = Path<usize>;

    async fn body(notes: &Notes, Path(id): Path<usize>) -> askama::Result<Body> {
        let text = notes.get(id);
        let found = text.is_some();
        let body = Body::new(Html::render(&NoteBody {
            id,
            text: text.clone(),
        })?)
        .title(text);
        Ok(if found {
            body
        } else {
            body.status(StatusCode::NOT_FOUND)
        })
    }
}

/// The newest note, wherever it is.
struct Latest;

impl Page for Latest {
    type Layout = Board;
    const PLACE: &'static str = "latest";
    type Args = ();

    async fn body(notes: &Notes, (): ()) -> askama::Result<Body> {
        Ok(match notes.matching("").first() {
            Some((id, _)) => Body::redirect(format!("/notes/{id}")),
            None => Body::redirect("/"),
        })
    }
}

/// A page with nothing live on it.
struct About;

impl Page for About {
    type Layout = Board;
    const PLACE: &'static str = "about";
    type Args = ();

    async fn body(_: &Notes, (): ()) -> askama::Result<Body> {
        Ok(Body::new(Html::trusted(
            "<p>Notes, kept until the server stops.</p>",
        )))
    }
}

// The actions.

#[derive(serde::Deserialize)]
struct NewNote {
    text: String,
}

/// Adds a note. Every page's stream draws it, so the answer only clears the
/// form, or says what is wrong.
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

/// Deletes a note, and shows the list it was in.
async fn delete(State(notes): State<Notes>, Path(id): Path<usize>) -> Response {
    notes.delete(id);
    match Index::body(&notes, View(Search::default())).await {
        Ok(body) => events(page::shown::<Index>(&notes, body, "/").await),
        Err(error) => Board::failed(&notes, error, true),
    }
}

// The live stream.

/// The board changing.
struct Changed(watch::Receiver<u64>);

impl Changes for Changed {
    type Changed = bool;

    fn everything(&self) -> bool {
        true
    }

    async fn changed(&mut self) -> Option<bool> {
        self.0.changed().await.ok().map(|()| true)
    }
}

/// The count in the menu, on every page.
struct Counter {
    notes: Notes,
    last: Last,
}

impl Draw<bool> for Counter {
    async fn draw(&mut self, changed: &bool) -> Vec<Event> {
        if !changed {
            return Vec::new();
        }
        self.last
            .send(&render(&Count::of(&self.notes)))
            .into_iter()
            .collect()
    }
}

/// The list, on the page that shows it, as its address's search finds it.
struct Listing {
    notes: Notes,
    q: Option<String>,
    last: Last,
}

impl Draw<bool> for Listing {
    async fn draw(&mut self, changed: &bool) -> Vec<Event> {
        let Some(q) = self.q.as_deref().filter(|_| *changed) else {
            return Vec::new();
        };
        let list = render(&List {
            notes: self.notes.matching(q),
        });
        self.last.send(&list).into_iter().collect()
    }
}

/// The signals the stream reads: the page's address.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Shown {
    url: String,
}

async fn live(State(notes): State<Notes>, Signals(shown): Signals<Shown>) -> Response {
    let on_index = shown.url.split('?').next() == Some("/");
    let View(search) = View::<Search>::of(&shown.url);
    let drawn = (
        Counter {
            notes: notes.clone(),
            last: Last::new(),
        },
        Listing {
            q: on_index.then_some(search.q),
            notes: notes.clone(),
            last: Last::new(),
        },
    );
    let changes = Changed(notes.changed.subscribe());
    stream(live::redraws(changes, drawn, Timing::default()))
}

pub(crate) fn routes(notes: Notes) -> Router {
    Router::new()
        .route("/notes", post(add))
        .route("/notes/{id}/delete", post(delete))
        .layer(middleware::from_fn(require_datastar_on_post))
        .route("/", get(page::serve::<Index>))
        .route("/notes/{id}", get(page::serve::<Note>))
        .route("/latest", get(page::serve::<Latest>))
        .route("/about", get(page::serve::<About>))
        .route("/live", get(live))
        .route("/datastar.js", get(script::serve))
        .with_state(notes)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, routes(Notes::default())).await
}
