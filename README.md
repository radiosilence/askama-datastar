# askama_datastar

[Askama](https://askama.rs) templates as [Datastar](https://data-star.dev) patches,
for [axum](https://docs.rs/axum).

The server renders HTML and Datastar patches it over server-sent events, so a page is
drawn by the same templates whether it is loaded whole or changed in place, and
reloading it loses nothing. This crate is the glue every such server needs, in pieces
that each work alone:

| Piece | What it does |
|---|---|
| `Html` | Markup a template rendered. A page embeds it as it is, `.patch()` sends it as a patch, and a handler returns it as `text/html` |
| `events`, `stream` | An answer of a few events, or a live stream kept open, neither stored by a cache |
| `Signals<T>` | The signals a Datastar request carries, drawn as the default view when they do not parse |
| `is_datastar`, `require_datastar_on_post` | Datastar's requests told from page loads; a POST it did not send refused |
| `reopen`, `reopen_on` | The attributes that open a page's live stream and open it again when it ends or goes quiet, or when signals it answers for change |
| `script` | Datastar's script, vendored and pinned, served under a fingerprinted URL |
| `Asset` | Any other file embedded in the binary, served under a fingerprinted URL |
| `Csp`, `SecurityHeaders` | The Content Security Policy every page starts from, and a layer setting it with the other security headers |
| `page` | Pages drawn whole for a browser or in place for the page itself, and an address the address bar follows |
| `live` | A live stream's loop: redrawn on change, throttled, sent only where it differs, ended after a lifetime |
| `clear` | A patch emptying a region the server filled |
| `escaping` | A guard for an application's tests: every answer escapes text exactly once |

## Install

```toml
[dependencies]
askama_datastar = "0.2"
```

`Html` implements `HtmlSafe` for the askama release this crate depends on. Under another
release `Html::render` does not compile and an `Html` in a template comes out escaped,
so a server's askama moves with this crate's.

## Quick start

A counter. The page renders the count; the button posts to a handler that renders the
same template and patches it into the page by its id. This is
[`examples/counter.rs`](examples/counter.rs): `cargo run --example counter`, then open
<http://127.0.0.1:3000>.

```rust
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use askama::Template;
use askama_datastar::{Html, events, require_datastar_on_post, script};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::Response,
    routing::{get, post},
};

#[derive(Template)]
#[template(ext = "html", source = r#"<p id="count">{{ n }}</p>"#)]
struct Count {
    n: u64,
}

#[derive(Template)]
#[template(
    ext = "html",
    source = r#"<!doctype html>
<script type="module" src="{{ script }}"></script>
{{ count }}
<button data-on:click="@post('/increment')">Add one</button>"#
)]
struct Page {
    script: String,
    count: Html,
}

type Counter = Arc<AtomicU64>;

async fn page(State(counter): State<Counter>) -> Result<Html, StatusCode> {
    let count = render(&Count {
        n: counter.load(Ordering::Relaxed),
    })?;
    render(&Page {
        script: script::url("/datastar.js"),
        count,
    })
}

async fn increment(State(counter): State<Counter>) -> Result<Response, StatusCode> {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    Ok(events([render(&Count { n })?.patch().into()]))
}

fn render(template: &impl Template) -> Result<Html, StatusCode> {
    Html::render(template).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = Router::new()
        .route("/increment", post(increment))
        .layer(middleware::from_fn(require_datastar_on_post))
        .route("/", get(page))
        .route("/datastar.js", get(script::serve))
        .with_state(Counter::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
```

The guard is layered before the page and the script are routed, so it covers only the
route Datastar posts to. A real server keeps its templates in `templates/` rather than
inline. The sections below take each piece in turn.

## A fragment, written once

A fragment is its own template, or a block of the page's. The page renders it in place,
and a handler sends the same template as a patch, which Datastar morphs into the page
by the ids of its top-level elements. There is never a second copy of the markup for
updates to drift from.

```rust
use askama::Template;
use askama_datastar::{Html, events};

#[derive(Template)]
#[template(path = "search/results.html")]
struct Results {
    rows: Vec<Row>,
}

#[derive(Template)]
#[template(path = "pages/search.html")] // `{{ results }}` where the results go
struct SearchPage {
    results: Html,
}

async fn page(State(app): State<App>) -> Result<Html, AppError> {
    let results = Html::render(&Results { rows: app.rows("") })?;
    Ok(Html::render(&SearchPage { results })?)
}

async fn results(State(app): State<App>, Signals(search): Signals<Search>) -> Result<Response, AppError> {
    Ok(events([Html::render(&Results { rows: app.rows(&search.q) })?.patch().into()]))
}
```

`Html` implements askama's `HtmlSafe`, so a template writes it unescaped with no
`|safe`, and escapes every other value. Markup the program writes itself, such as an
icon, is `Html::trusted`. For a fragment kept inside the page's own template, askama's
`blocks = ["results"]` attribute gives the page an `as_results()` that renders the
block alone, and `Html::render(&page.as_results())` takes it like any template.

A patch targets something other than the ids with the SDK's builder methods:

```rust
Html::render(&row)?.patch().selector("#rows").mode(ElementPatchMode::Append)
```

### Where ids go

Datastar morphs a patch into the page by the ids of its top-level elements, so
an id is a promise that the server sends that region again. An id goes only on
a region the server sends again, and is declared once, in that region's own
template, so the page and the patch cannot disagree on it. One region sent
whole is preferred to many small ones: it is one template, one id and one
comparison in a stream. A repeated control that changes everywhere at once,
such as a button naming one person on every row that does, is found by a
`data-` attribute rather than by ids of its own. A form carries an id only
where something outside it names it, as an input's `form=` does.

A region the server filled that the person closes, such as a preview, is
emptied with `clear("#preview")`, which keeps the element and replaces what is
inside it with nothing. Sending a region with no markup by id would change
nothing, since there is no element to match. A region whose closed state has
markup of its own is a template sent like any other.

## Answers and streams

`events` answers with patches and signal updates together, in order:

```rust
events([
    Html::render(&flash)?.patch().into(), // axum's SSE `Event`, which each kind converts into
    PatchSignals::new(r#"{"name":""}"#).into(),
])
```

`events(Vec::new())` answers without changing the page.

An answer of fragments alone can be the `Html` itself, as `text/html`. Datastar morphs
each of its top-level elements into the page by id, as it does a patch, and a
compression layer compresses `text/html` where it leaves an event stream alone, so a
large fragment travels smaller this way. The response varies on `Datastar-Request`, so
one address can answer a page load with the page and Datastar with a fragment without
a browser's cache showing one in place of the other.

`stream` keeps a live stream open, sending each event as it comes, and a heartbeat
(`event: datastar-heartbeat`) after 15 seconds of quiet, so a proxy does not close the
connection and the page can tell a quiet stream from a lost one:

```rust
stream(async_stream::stream! {
    let mut changed = jobs.subscribe();
    loop {
        yield Html::render(&Status::of(&jobs)).unwrap_or_default().patch();
        if changed.changed().await.is_err() {
            break;
        }
    }
})
```

Both send `Cache-Control: no-store`: what they carry is the page as one person sees it.

### Keeping a stream open

A stream ends when the server stops. Datastar retries a broken connection but does not
reopen a stream that ended, and `retry: 'always'` counts each reopening against its ten
retries; a socket that stops carrying bytes without an error, as a suspended laptop's
does, looks live to it for good. So a page holding a live stream reopens it itself.
`reopen(url)` writes the attributes that do, for an element of their own that stays on
the page:

```rust
#[derive(Template)]
#[template(ext = "html", source = "<body>…<div hidden{{ live }}></div></body>")]
struct Page {
    live: Html, // reopen("/stream")
}
```

The element makes no other requests: the attributes take every request it makes for the
stream, so a POST from it would hold the stream open or, refused, stop it. They open
the stream a moment after the page is set up, so the first request carries the page's
signals, and every 5 seconds while the page is shown open it again if it ended or has
said nothing for 40 seconds, two heartbeats' worth. A stream refused with 401 or 403,
such as a session a deploy ended, is not asked again until the page is shown after
being hidden; any other status is taken for temporary. Returning to a hidden page
reopens a stream that ended meanwhile; one still open Datastar reopens itself. `@get`
aborts the request already open to the same URL, so there is only ever one stream.

No script of the page's own is needed: the expressions keep their state in
`el.__reopen`, and the URL is written into `data-reopen-url` and read with
`el.dataset.reopenUrl`, never into an expression, so no URL can inject script.
`reopen` takes only a path on the page's own origin, and panics on anything else.

A stream that answers for some of the page's signals, such as the tab shown, is opened
afresh when they change: `reopen_on("/stream", &["tab", "filters"])` adds an effect that
reads them. The effect skips its first run, so the stream is opened once as the page
starts rather than by the effect and again by the delayed start, which would abort the
first request on every page load.

## Pages and their streams

An application whose pages each have an address draws a page two ways: whole,
for a browser loading the address, and in place, for the page following a link
without leaving the document. Both are drawn from one body, so a reload, a
shared link, back and forward show what the address names. A live stream keeps
the page current. The pieces for this are small and each works alone; together
they make the routed application in
[`examples/notes.rs`](examples/notes.rs), a notes board whose list updates in
every open tab: `cargo run --example notes`. The excerpts below are from it.

### Pages

The application writes its document once, as a `page::Layout`, and decides
everything in it: what a page's `Place` is (here, the menu entry it is shown
under), which signals every page has, which stream it holds open, and how a
failure is shown.

```rust
struct Board;

impl Layout for Board {
    type State = Notes;
    type Place = &'static str;
    type Error = askama::Error;

    async fn whole(notes: &Notes, entry: &&'static str, mut page: Whole) -> askama::Result<Html> {
        page.signals.entry("text").or_insert("".into());
        Html::render(&Document {
            title: render(&Title { name: page.title.as_deref().unwrap_or(entry) }),
            attributes: page.attributes(), // <body{{ attributes }}>
            live: reopen_on("/live", &[Self::ADDRESS]),
            body: page.body,               // <main id="main">{{ body }}</main>
            // …
        })
    }

    async fn in_place(_: &Notes, entry: &&'static str, title: Option<&str>) -> Vec<Event> {
        let title = render(&Title { name: title.unwrap_or(entry) });
        vec![title.patch().into(), clear("#flash").into()]
    }

    fn failed(_: &Notes, error: askama::Error, in_place: bool) -> Response { … }
}
```

Each page is a `page::Page`: its place, what it reads from the request (any
extractor), and its `Body`.

```rust
struct Note;

impl Page for Note {
    type Layout = Board;
    const PLACE: &'static str = "notes";
    type Args = Path<usize>;

    async fn body(notes: &Notes, Path(id): Path<usize>) -> askama::Result<Body> {
        let text = notes.get(id);
        let found = text.is_some();
        let body = Body::new(Html::render(&NoteBody { id, text: text.clone() })?).title(text);
        Ok(if found { body } else { body.status(StatusCode::NOT_FOUND) })
    }
}

let app = Router::new()
    .route("/", get(page::serve::<Index>))
    .route("/notes/{id}", get(page::serve::<Note>));
```

`page::serve` answers a browser with the layout's document around the body,
with the body's status. It answers the page itself, loading the address in
place, with the body morphed into `Layout::MAIN` (`#main` by default), the
layout's `in_place` events, and the address in the layout's `ADDRESS` signal
(`url` by default). `Whole::attributes` writes the page's signals and
`page::history`, which pushes the address onto the history when the signal
changes, and loads the page whole when back or forward reaches an address it
does not show. A link loads in place with a constant expression,
`data-on:click__prevent="@get(el.href)"`, and a search form with
`@get('/', {contentType: 'form'})`, which puts its fields in the address.

`Body::redirect(path)` is for an address that now names another page: a
browser is redirected, and a page loading in place goes there with
`page::go`, a patch that loads an address whole. An action that leads to
another page answers with `page::shown::<P>(state, body, address)`, the same
events an in-place load gets.

The pieces work without the traits: `page::View<T>` reads an address's query
leniently (a query that does not parse is `T::default()`, since an address is
something a person types and keeps), as an extractor or with `View::of(url)`;
`page::address(&uri)` is a request's address less Datastar's own parameter;
`page::history(signal)` and `page::go(path)` stand alone.

### Live streams

A live stream is `live::redraws(changes, draw, timing)`, sent with `stream`.
`changes` is where the application's changes come from, a `live::Changes`;
`draw` draws the regions a change touches, a `live::Draw`, and a pair of
drawers is one drawer, so a region every page shows and a region of one page
are written apart. The handler is the application's own:

```rust
async fn live(State(notes): State<Notes>, Signals(shown): Signals<Shown>) -> Response {
    let on_index = shown.url.split('?').next() == Some("/");
    let View(search) = View::<Search>::of(&shown.url);
    let drawn = (
        Counter { notes: notes.clone(), last: Last::new() },
        Listing { q: on_index.then_some(search.q), notes: notes.clone(), last: Last::new() },
    );
    stream(live::redraws(Changed(notes.changed.subscribe()), drawn, Timing::default()))
}
```

It draws everything once, then waits for a change, or for a drawer's own
`wake` (a reply it asked for), and draws again, at most every
`Timing::throttle` (250 ms). A source that reports what changed since it was
last asked, as a `tokio::sync::watch` does, gathers a burst into one redraw.
`Last` sends a region only when it differs from what was last sent, so a
redraw that changes nothing sends nothing; `Last::comparing(f)` compares what
`f` keeps of it, for markup that names something afresh on every render.

The stream ends after `Timing::lifetime` (a minute), and the page's `reopen`
attributes open another within seconds: a stream can die without either end
noticing, as a suspended phone's does, and the lifetime bounds how long a page
sits on one. The stream draws the page its address signal names and never
follows navigation; `reopen_on` that signal opens it afresh when the page
moves.

## The escaping guard

A fragment spliced into a template as text, rather than as `Html`, comes out
escaped, and the page shows its own markup as raw HTML; text spliced in as
markup comes out unescaped, and runs. A page checked with ordinary text in it
shows neither. `escaping::Guard` checks answers drawn with markup-shaped text
in every field a person controls. From the notes example's test
([`tests/notes.rs`](tests/notes.rs)):

```rust
const HOSTILE: &str = r#"<em>hostile "q" & 'x'</em>"#;

let id = board.add(HOSTILE);
let guard = Guard::new(&[HOSTILE]).markup(include_str!("../examples/notes.rs"));
for (url, shows) in [("/", true), (&search[..], true), (&note[..], true), ("/about", false)] {
    for in_place in [false, true] {
        let (status, answer) = get(&router, url, in_place, WAIT).await;
        found.extend(guard.problems(&answer).iter().map(|p| format!("{url}: {p}")));
        if shows && !answer.contains(&escaped(HOSTILE)) {
            found.push(format!("{url}: the note is missing"));
        }
    }
    let (_, answer) = get(&router, &live(url), true, WAIT).await;
    found.extend(guard.problems(&answer).iter().map(|p| format!("{url} live: {p}")));
}
```

`Guard::problems` finds markup of the application's own escaped (`&#60;` or
`&lt;` before an element name it writes), text escaped twice (`&#38;#60;`,
`&amp;lt;`), and the hostile text left as markup outside an event stream's
`data: signals` lines, which are JSON. The element names come from wherever
the markup lives: `Guard::templates(dir)` reads a templates directory,
`Guard::markup(text)` any markup, `Guard::elements(names)` names given.
`escaped` is askama's escaping, the form a page shows text in. `get` reads an
answer through the router, cutting a live stream off after the wait; a stream
is asked for with its signals in the `datastar` query parameter.

## Requests and signals

Datastar marks every request with a `Datastar-Request` header, which a cross-site form
cannot set. `is_datastar` reads it: a page load wants a whole page, and a request the
page makes wants a patch, or a status where a page load would be redirected, since
Datastar follows a redirect like any fetch.

`require_datastar_on_post` is middleware refusing, with 403, a POST without the header.
Layered over the routes Datastar posts to, it stops cross-site request forgery whatever
the cookie's `SameSite` allows. POST is the one method that needs it: a form sends only
GET and POST, and another site's script cannot send PUT, PATCH or DELETE without a CORS
preflight the server never answers. A plain form, such as a sign-in, posts to a route
outside it, whose handler checks `Sec-Fetch-Site` or `Origin` itself.

`Signals<T>` reads the signals Datastar sends: the `datastar` query parameter of a GET
or DELETE, and the JSON body of anything else. Signals that are missing or do not parse
read as `T::default()`, since they describe what a page shows, and a view whose state
does not parse is drawn as its default rather than refused. A request that changes
something should refuse what it cannot read instead: use the SDK's `ReadSignals`
(`askama_datastar::datastar::axum::ReadSignals`) or axum's `Json` there, into a struct
that accepts fields it does not name, since Datastar sends every signal on the page.

## The script

```rust
let app = Router::new().route("/assets/datastar.js", get(askama_datastar::script::serve));
let src = askama_datastar::script::url("/assets/datastar.js"); // for <script type="module" src="…">
```

`assets/datastar.js` is Datastar's `bundles/datastar.js` at the release
`script::VERSION` names, byte for byte. Asked for by the URL `script::url` gives, which
carries a hash of the script, it is cached for good; any other way, for an hour.
Datastar compiles its attribute expressions with `Function`, so a page that loads it
needs `'unsafe-eval'` in its `script-src`. Because the script is served from the
binary, nothing else is needed: `script-src 'self' 'unsafe-eval'`, which
`Csp::datastar` gives.

To move to a new Datastar release, run `just datastar X.Y.Z`, which writes the script
and `VERSION`, and add a changelog fragment saying what the release changes. CI
downloads the release again and fails if the vendored copy is not exactly it.

## Assets

Every other file a page loads goes the same way as the script: embedded, and named by a
hash of its bytes so a browser keeps it for good and a new build's page fetches the new
copy.

```rust
static APP_CSS: Asset = Asset::new(include_bytes!("../assets/app.css"), "text/css; charset=utf-8");

let app = Router::new().route("/assets/app.css", get(APP_CSS.serve()));
let href = APP_CSS.url("/assets/app.css"); // "/assets/app.css?v=…"
```

The hash is FNV-1a over the bytes, worked out once. `APP_CSS.response(query)` answers
from a handler serving several files from one route.

## Security headers

`SecurityHeaders` is a layer setting, on every response, a Content Security Policy,
`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` and
`Referrer-Policy: same-origin`. `Csp::new()` is the policy every page starts from:

```text
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self';
connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'
```

What a server adds to it is one call each:

```rust
let csp = Csp::new()
    .datastar()               // script-src 'self' 'unsafe-eval'
    .import_map(&map_json)    // script-src 'sha256-…' of the map's exact text
    .allow("img-src", "blob:"); // previews of files about to be uploaded
let app = Router::new().route("/", get(page)).layer(SecurityHeaders::new(&csp));

let sign_in = Csp::new().deny("script-src"); // a page that runs nothing
```

The layer sets a header only where the response lacks it, so a route needing a stricter
or wider policy, such as a file served with `sandbox`, sets its own
(`csp.header_value()`) and keeps it. `X-Frame-Options: DENY` repeats
`frame-ancestors 'none'` for older browsers, so it is left off when the policy allows a
frame ancestor and when the route set its own policy. The referrer policy is
`same-origin`, not `no-referrer`: under `no-referrer` a browser sends `Origin: null` on
a form's POST, which defeats the `Origin` check a sign-in form relies on.
`form-action` applies to redirects too, so a POST answered with a redirect to another
origin needs that origin allowed. Nothing adds `'unsafe-inline'`. With eval allowed,
escaping is what keeps text from running, so text stays out of Datastar expressions and
in `data-*` attributes read with `el.dataset`.

## Why not the SDK alone

Datastar's own Rust SDK, [`datastar`](https://docs.rs/datastar), writes the events, and
this crate builds on it (re-exported as `askama_datastar::datastar`) rather than
writing its own. What the SDK leaves to each server is what this adds: rendering
Askama templates into markup a page can embed, markup that cannot break an SSE line
(the SDK splits markup on `\n` alone, and axum ends a line at a lone `\r` too, so the
rest of that line would reach Datastar without its `elements` prefix), answers that no
cache keeps, the script itself, and signals read leniently for a view where the SDK's
`ReadSignals` refuses them with 400.

## Development

```sh
just check   # format, lint, tests, docs, changelog, workflows
just fmt
```

Every change adds a changelog fragment under `changelog.d/unreleased/<section>/`.
`CHANGELOG.md` is generated from them (see `changelog.d/README.md`) and never edited by
hand. `just release patch|minor|major` opens the release pull request, and merging it
publishes the crate and the GitHub release.

`.github/workflows/ci-cd.yml` ends in `Checks`, the one status the ruleset requires.
Each job runs only when the change touches what it checks, and a skipped job passes
`Checks`. A new job needs adding to `Checks`'s `needs`; a new top-level directory of
code needs adding to the `rust` pattern in the `changes` job.

## Licence

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT),
at your option. `assets/datastar.js` is Datastar's, under its
[MIT licence](https://github.com/starfederation/datastar/blob/develop/LICENSE.md).
