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
| `script` | Datastar's script, vendored and pinned, served under a fingerprinted URL |

## Install

```toml
[dependencies]
askama_datastar = "0.1"
```

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
    Ok(events([Html::render(&Results { rows: app.rows(&search.q) })?.patch()]))
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
    Html::render(&flash)?.patch().into(),
    PatchSignals::new(r#"{"name":""}"#).into(),
])
```

`stream` keeps a live stream open, sending each event as it comes, and a comment after
15 seconds of quiet so a proxy does not close the connection:

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

## Requests and signals

Datastar marks every request with a `Datastar-Request` header, which a cross-site form
cannot set. `is_datastar` reads it: a page load wants a whole page, and a request the
page makes wants a patch, or a status where a page load would be redirected, since
Datastar follows a redirect like any fetch.

`require_datastar_on_post` is middleware refusing, with 403, a POST without the header.
Every state change is a POST made by Datastar, so that stops cross-site request forgery
whatever the cookie's `SameSite` allows.

`Signals<T>` reads the signals Datastar sends: the `datastar` query parameter of a GET
or DELETE, and the JSON body of anything else. Signals that are missing or do not parse
read as `T::default()`, since they describe what a page shows, and a view whose state
does not parse is drawn as its default rather than refused. A request that changes
something should refuse what it cannot read instead: use the SDK's `ReadSignals` or
axum's `Json` there.

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
binary, nothing else is needed: `script-src 'self' 'unsafe-eval'`.

To move to a new Datastar release, run `just datastar X.Y.Z`, which writes the script
and `VERSION`, and add a changelog fragment saying what the release changes. CI
downloads the release again and fails if the vendored copy is not exactly it.

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
