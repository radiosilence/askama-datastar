# Changelog

## 0.1.0

The first release: the glue between Askama, Datastar and axum that radiosilence's servers each wrote for themselves, built on Datastar's Rust SDK.

### Added

- **Markup that pages embed and patches send.** `Html` holds what a template rendered: a page embeds it without `|safe`, `Html::patch` sends the same markup as a Datastar patch, and a handler can return it as `text/html`, so a fragment is written once. Line breaks are normalised as an HTML parser normalises them, so a carriage return in the markup cannot end an SSE line early and drop the rest of it.
- **Datastar's requests and signals.** `is_datastar` tells Datastar's requests from page loads, `require_datastar_on_post` refuses a POST without Datastar's header, and `Signals` reads a request's signals, drawing the default view when they are missing or do not parse.
- **Datastar v1.0.5, vendored.** `script::serve` serves Datastar's script from the binary and `script::url` names it with a hash of its contents, so a browser keeps it for good and fetches a new release at once. CI checks the copy byte for byte against the release's.
- **Answers and live streams.** `events` answers with patches and signal updates in order; `stream` keeps a live stream open, with a comment after 15 seconds of quiet so a proxy leaves it open. Neither may be kept by a cache.
