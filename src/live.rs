//! The loop of a live stream that keeps a page current.
//!
//! A stream of this kind draws everything once, then waits for a change and
//! draws what it touched, again and again. The pieces:
//!
//! - [`Changes`] is where changes come from: the application's, written once.
//! - [`Draw`] draws the regions a change touches. A pair of drawers is a
//!   drawer, so regions every page shows and those of one page are written
//!   apart and run together.
//! - [`redraws`] is the loop: it draws at most every [`Timing::throttle`], so
//!   a burst of changes is one redraw, and ends after [`Timing::lifetime`].
//! - [`Last`] sends a region only when it differs from what was last sent.
//!
//! The handler is the application's own, so it reads whatever it needs from
//! the request:
//!
//! ```
//! # use askama_datastar::{Html, stream};
//! # use askama_datastar::live::{self, Changes, Draw, Last, Timing};
//! # use axum::{extract::State, response::{Response, sse::Event}};
//! # use tokio::sync::watch;
//! # #[derive(Clone)] struct App { count: std::sync::Arc<watch::Sender<u64>> }
//! /// The count changing.
//! struct Counted(watch::Receiver<u64>);
//!
//! impl Changes for Counted {
//!     type Changed = bool;
//!     fn everything(&self) -> bool {
//!         true
//!     }
//!     async fn changed(&mut self) -> Option<bool> {
//!         self.0.changed().await.ok().map(|()| true)
//!     }
//! }
//!
//! /// The count, as the page shows it.
//! struct Count(watch::Receiver<u64>, Last);
//!
//! impl Draw<bool> for Count {
//!     async fn draw(&mut self, _: &bool) -> Vec<Event> {
//!         let count = Html::trusted(format!(r#"<p id="count">{}</p>"#, *self.0.borrow()));
//!         self.1.send(&count).into_iter().collect()
//!     }
//! }
//!
//! async fn live(State(app): State<App>) -> Response {
//!     let count = Count(app.count.subscribe(), Last::new());
//!     stream(live::redraws(Counted(app.count.subscribe()), count, Timing::default()))
//! }
//! ```
//!
//! Ending on a timer matters: a stream can die without either end noticing (a
//! suspended phone, a replaced server), and the lifetime bounds how long a
//! page sits on one. The page's [`reopen`](crate::reopen) attributes open
//! another within seconds. A stream that draws the page an address names need
//! not follow navigation: [`reopen_on`](crate::reopen_on) the signal holding
//! the address opens it afresh when the page moves.

use std::future::Future;
use std::time::Duration;

use axum::response::sse::Event;
use futures_util::{Stream, StreamExt as _};
use tokio::time::{Instant, sleep, sleep_until};

use crate::Html;

/// Where a stream's changes come from.
///
/// A source that reports what changed since it was last asked, as a
/// `tokio::sync::watch` does, gathers a burst of changes into one by itself.
pub trait Changes: Send + 'static {
    /// What changed. Its default is that nothing did, which is what a drawer
    /// waking for a change of its own ([`Draw::wake`]) draws with.
    type Changed: Default + Send + Sync;

    /// What the first draw is told: everything changed.
    fn everything(&self) -> Self::Changed;

    /// The next change, or `None` once the source has gone, which ends the
    /// stream. It must be safe to drop before it resolves, as
    /// `tokio::select!` needs.
    fn changed(&mut self) -> impl Future<Output = Option<Self::Changed>> + Send;
}

/// What draws the regions a change touches.
pub trait Draw<C>: Send + 'static {
    /// The events for the regions `changed` touches. Everything, the first
    /// time.
    fn draw(&mut self, changed: &C) -> impl Future<Output = Vec<Event>> + Send;

    /// Resolves when the drawer has a change of its own to draw, such as a
    /// reply it asked for; it is then drawn with nothing else changed. Never,
    /// unless the drawer says. It must be safe to drop before it resolves.
    fn wake(&mut self) -> impl Future<Output = ()> + Send {
        std::future::pending()
    }
}

/// Two drawers as one: the first's events, then the second's. Either waking
/// wakes both.
impl<C: Sync, A: Draw<C>, B: Draw<C>> Draw<C> for (A, B) {
    async fn draw(&mut self, changed: &C) -> Vec<Event> {
        let mut events = self.0.draw(changed).await;
        events.extend(self.1.draw(changed).await);
        events
    }

    async fn wake(&mut self) {
        tokio::select! {
            () = self.0.wake() => {}
            () = self.1.wake() => {}
        }
    }
}

/// How often a stream redraws, and how long it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// The least time between two redraws. 250 ms by default.
    pub throttle: Duration,
    /// How long the stream runs before the page opens another. A minute by
    /// default.
    pub lifetime: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            throttle: Duration::from_millis(250),
            lifetime: Duration::from_secs(60),
        }
    }
}

/// The events of `draw` redrawing on every change `changes` reports, starting
/// with everything, at most every `timing.throttle`, until the source goes or
/// `timing.lifetime` passes. [`stream`](crate::stream) sends them.
pub fn redraws<C, D>(changes: C, draw: D, timing: Timing) -> impl Stream<Item = Event> + Send
where
    C: Changes,
    D: Draw<C::Changed>,
{
    let run = Run {
        changed: Some(changes.everything()),
        changes,
        draw,
        timing,
        ends: Instant::now() + timing.lifetime,
    };
    futures_util::stream::unfold(run, Run::next).flat_map(futures_util::stream::iter)
}

struct Run<C: Changes, D> {
    changes: C,
    draw: D,
    timing: Timing,
    ends: Instant,
    /// What the next draw is told, when it is known before waiting.
    changed: Option<C::Changed>,
}

impl<C: Changes, D: Draw<C::Changed>> Run<C, D> {
    async fn next(mut self) -> Option<(Vec<Event>, Self)> {
        let changed = if let Some(changed) = self.changed.take() {
            changed
        } else {
            sleep(self.timing.throttle).await;
            tokio::select! {
                changed = self.changes.changed() => changed?,
                () = self.draw.wake() => C::Changed::default(),
                () = sleep_until(self.ends) => return None,
            }
        };
        let events = self.draw.draw(&changed).await;
        Some((events, self))
    }
}

/// A region as last sent, to send it again only when it differs.
///
/// Markup that names something afresh on every render from the same state,
/// such as a generated id, is compared by what stays put:
/// [`Last::comparing`] takes the function that keeps it.
#[derive(Clone, Debug, Default)]
pub struct Last {
    sent: Option<String>,
    compare: Option<fn(&str) -> String>,
}

impl Last {
    /// Nothing sent yet; markup is compared as it is.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sent: None,
            compare: None,
        }
    }

    /// Nothing sent yet; markup is compared as `compare` gives it.
    #[must_use]
    pub const fn comparing(compare: fn(&str) -> String) -> Self {
        Self {
            sent: None,
            compare: Some(compare),
        }
    }

    /// `html` as a patch by its ids, unless it is what was last sent.
    pub fn send(&mut self, html: &Html) -> Option<Event> {
        let compared = match self.compare {
            Some(compare) => compare(html.as_str()),
            None => html.as_str().to_owned(),
        };
        if self.sent.as_ref() == Some(&compared) {
            return None;
        }
        self.sent = Some(compared);
        Some(html.patch().into())
    }
}

#[cfg(test)]
mod tests {
    use super::Last;
    use crate::Html;

    #[test]
    fn a_region_is_sent_again_only_when_it_differs() {
        let mut last = Last::new();
        assert!(last.send(&Html::trusted("<p id=a>1</p>")).is_some());
        assert!(last.send(&Html::trusted("<p id=a>1</p>")).is_none());
        assert!(last.send(&Html::trusted("<p id=a>2</p>")).is_some());
    }

    #[test]
    fn a_region_can_be_compared_by_what_stays_put() {
        let mut last =
            Last::comparing(|html| html.split(" data-n").next().unwrap_or(html).to_owned());
        assert!(last.send(&Html::trusted("<p id=a data-n=1>")).is_some());
        assert!(last.send(&Html::trusted("<p id=a data-n=2>")).is_none());
    }
}
