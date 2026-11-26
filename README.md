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
| `reopen` | The attributes that open a page's live stream and open it again when it ends or goes quiet |
| `script` | Datastar's script, vendored and pinned, served under a fingerprinted URL |
| `Asset` | Any other file embedded in the binary, served under a fingerprinted URL |
| `Csp`, `SecurityHeaders` | The Content Security Policy every page starts from, and a layer setting it with the other security headers |

## Install

```toml
[dependencies]
askama_datastar = "0.1"
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
`reopen(url)` writes the attributes that do, for an element that stays on the page:

```rust
#[derive(Template)]
#[template(ext = "html", source = "<body{{ live }}>…</body>")]
struct Page {
    live: Html, // reopen("/stream")
}
```

They open the stream, and every 5 seconds while the page is shown open it again if it
ended or has said nothing for 40 seconds, two heartbeats' worth. A stream the server
refused with a 4xx, such as the 401 of a session a deploy ended, is not asked again
until the page is shown after being hidden. Returning to a hidden page reopens a stream
that ended meanwhile; one still open Datastar reopens itself. `@get` aborts the request
already open to the same URL, so there is only ever one stream. No script of the page's
own is needed: the expressions keep their state on the element, and the URL is written
into `data-url` and read with `el.dataset.url`, never into an expression, so any URL
is safe there.

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
`Referrer-Policy: no-referrer`. `Csp::new()` is the policy every page starts from:

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
(`csp.header_value()`) and keeps it. Nothing adds `'unsafe-inline'`. With eval allowed,
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
