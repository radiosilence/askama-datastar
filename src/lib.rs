//! [Askama] templates as [Datastar] patches, for [axum].
//!
//! The server renders HTML and Datastar patches it over server-sent events, so
//! a page is drawn by the same templates whether it is loaded whole or changed
//! in place, and reloading it loses nothing. This crate is the glue every such
//! server needs between the three, in pieces that each work alone:
//!
//! - [`Html`] is markup a template rendered. A page embeds it as it is, and
//!   [`Html::patch`] sends the same markup as a patch, so a fragment is written
//!   once.
//! - [`events`] answers a request with a few events; [`stream`] keeps a live
//!   stream open.
//! - [`Signals`] reads the signals a Datastar request carries.
//! - [`is_datastar`] tells Datastar's requests from page loads, and
//!   [`require_datastar_on_post`] refuses a POST that did not come from one.
//! - [`script`] serves Datastar's own script, vendored and pinned, under a
//!   fingerprinted URL.
//!
//! The events are written by Datastar's SDK ([`datastar`], re-exported), so
//! their wire format is the SDK's, which Datastar's SDK test suite checks.
//!
//! A whole server, a counter in one file, is
//! [`examples/counter.rs`](https://github.com/radiosilence/askama-datastar/blob/main/examples/counter.rs).
//!
//! # A fragment, written once
//!
//! A fragment is its own template. The page renders it in place, and a handler
//! sends the same template as a patch, which Datastar morphs into the page by
//! the ids of its top-level elements:
//!
//! ```
//! use askama::Template;
//! use askama_datastar::{Html, events};
//! use axum::response::Response;
//!
//! #[derive(Template)]
//! #[template(ext = "html", source = r#"<ul id="results">
//! {%- for row in rows %}<li>{{ row }}</li>{% endfor -%}
//! </ul>"#)]
//! struct Results {
//!     rows: Vec<String>,
//! }
//!
//! #[derive(Template)]
//! #[template(ext = "html", source = "<main>{{ results }}</main>")]
//! struct Page {
//!     results: Html,
//! }
//!
//! fn page(rows: Vec<String>) -> askama::Result<String> {
//!     Page { results: Html::render(&Results { rows })? }.render()
//! }
//!
//! fn search(rows: Vec<String>) -> askama::Result<Response> {
//!     Ok(events([Html::render(&Results { rows })?.patch().into()]))
//! }
//!
//! let rows = vec!["<b>moss</b>".to_owned()];
//! assert_eq!(
//!     page(rows)?,
//!     r#"<main><ul id="results"><li>&#60;b&#62;moss&#60;/b&#62;</li></ul></main>"#,
//! );
//! # Ok::<(), askama::Error>(())
//! ```
//!
//! Askama's `blocks` attribute does the same for a fragment kept inside the
//! page's own template: `page.as_results()` renders the `results` block alone,
//! and [`Html::render`] takes it like any template.
//!
//! # A live stream
//!
//! ```
//! use askama_datastar::{Html, stream};
//! use axum::response::Response;
//! use futures_util::{Stream, StreamExt as _};
//!
//! /// Each new status line, patched in as it changes.
//! fn follow(statuses: impl Stream<Item = Html> + Send + 'static) -> Response {
//!     stream(statuses.map(|status| status.patch()))
//! }
//! ```
//!
//! # Signals
//!
//! ```
//! use askama_datastar::Signals;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize, Default)]
//! #[serde(default)]
//! struct Search {
//!     q: String,
//! }
//!
//! async fn results(Signals(search): Signals<Search>) -> String {
//!     search.q
//! }
//! ```
//!
//! # The script and the guard
//!
//! ```
//! use axum::Router;
//! use axum::middleware::from_fn;
//! use axum::routing::{get, post};
//!
//! // A layer covers the routes added before it: here, the one Datastar posts to.
//! let app: Router = Router::new()
//!     .route("/save", post(|| async { "saved" }))
//!     .layer(from_fn(askama_datastar::require_datastar_on_post))
//!     .route("/assets/datastar.js", get(askama_datastar::script::serve));
//!
//! // What the page's `<script type="module" src="…">` names.
//! let src = askama_datastar::script::url("/assets/datastar.js");
//! assert!(src.starts_with("/assets/datastar.js?v="));
//! ```
//!
//! [Askama]: https://askama.rs
//! [Datastar]: https://data-star.dev
//! [axum]: https://docs.rs/axum

mod html;
mod request;
pub mod script;
mod sse;

pub use html::Html;
pub use request::{Signals, is_datastar, require_datastar_on_post};
pub use sse::{events, stream};

/// Datastar's SDK, which writes the events. Re-exported so a caller can name
/// its types (`ExecuteScript`, `ReadSignals`, `Namespace`) without depending
/// on the same release itself.
pub use datastar;
/// The SDK's events and the modes a patch takes, which [`Html::patch`]
/// returns and its builder methods set.
pub use datastar::prelude::{ElementPatchMode, PatchElements, PatchSignals};
