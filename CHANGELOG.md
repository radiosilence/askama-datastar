# Changelog

## 0.2.1

### Added

- **`reopen_on(url, signals)`: a live stream that also follows signals.** It adds an effect that opens the stream afresh when any of the signals changes, and skips the effect's first run, so the stream opens once as the page starts. A page that wrote such an effect beside `reopen` opened it twice, the second request aborting the first.

## 0.2.0

### Added

- **`Asset`: any embedded file under a fingerprinted URL.** What `script` does for Datastar's script, for a server's own stylesheet, scripts, fonts and icons: `url(path)` adds the FNV-1a hash of the bytes as its `v` parameter, and the file is cached for good at that address and for an hour at any other. `script` is now built on it.
- **`reopen(url)`: a live stream that reopens itself.** Datastar does not reopen a stream that ended, and cannot see one whose socket went silent, so pages wrote reopen attributes by hand. `reopen` writes them for an element of their own: the stream is opened again once it ends or has said nothing for 40 seconds, not after a 401 or 403 until the page is shown again, and the URL, a path on the page's origin, is read from `data-reopen-url` rather than written into an expression.
- **`Csp` and `SecurityHeaders`.** Every server wrote the same security middleware and the same policy by hand. `Csp::new()` is the policy STANDARD §8 gives, with `datastar()` for Datastar's `'unsafe-eval'`, `import_map(text)` for an import map's hash, `deny(directive)` for `script-src 'none'` and the like, and `allow(directive, source)` for extra sources such as `blob:`. `SecurityHeaders` is a layer setting it with `nosniff`, `X-Frame-Options: DENY` (while the policy refuses frames) and `Referrer-Policy: same-origin`, which keeps a form POST's `Origin` header where `no-referrer` would send `null`; it leaves a header a route set itself alone.

### Changed

- **A quiet stream sends a heartbeat event rather than a comment.** `stream` sends `event: datastar-heartbeat` after 15 seconds without an event, where it sent an SSE comment, which Datastar never shows the page. Datastar dispatches the event and runs nothing for it, so pages are unchanged; `reopen` hears it, and tells a quiet stream from a lost one by it.

## 0.1.2

### Changed

- **`#[must_use]` on `Html::trusted`, `events` and `stream`.** Dropping what they return is always a mistake.

### Fixed

- **A page and a fragment at one address stay apart in a cache.** `Html`'s response varies on `Datastar-Request`, so a browser going back to an address that answered Datastar with a fragment shows the page, not the fragment.

### Internal

- **The scripts are shellchecked.** `just check` and CI's Actionlint job run shellcheck over `scripts/*.sh`.

## 0.1.1

### Added

- **Quick start.** The README opens with a whole counter server, `examples/counter.rs`; clippy builds it and a test fails when the README's copy differs.

### Fixed

- **The docs no longer promise more than the crate does.** `require_datastar_on_post` is layered over the routes Datastar posts to, with plain forms outside it, where the docs said every state change was Datastar's; a live stream ends when its server stops and its page reopens it, since Datastar does not; an action's signals struct accepts fields it does not name; and a server's askama moves with this crate's.

## 0.1.0

The first release: the glue between Askama, Datastar and axum that radiosilence's servers each wrote for themselves, built on Datastar's Rust SDK.

### Added

- **Markup that pages embed and patches send.** `Html` holds what a template rendered: a page embeds it without `|safe`, `Html::patch` sends the same markup as a Datastar patch, and a handler can return it as `text/html`, so a fragment is written once. Line breaks are normalised as an HTML parser normalises them, so a carriage return in the markup cannot end an SSE line early and drop the rest of it.
- **Datastar's requests and signals.** `is_datastar` tells Datastar's requests from page loads, `require_datastar_on_post` refuses a POST without Datastar's header, and `Signals` reads a request's signals, drawing the default view when they are missing or do not parse.
- **Datastar v1.0.5, vendored.** `script::serve` serves Datastar's script from the binary and `script::url` names it with a hash of its contents, so a browser keeps it for good and fetches a new release at once. CI checks the copy byte for byte against the release's.
- **Answers and live streams.** `events` answers with patches and signal updates in order; `stream` keeps a live stream open, with a comment after 15 seconds of quiet so a proxy leaves it open. Neither may be kept by a cache.
