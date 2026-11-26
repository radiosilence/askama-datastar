//! A live stream's pace, and the regions it has already sent.
//!
//! A live stream redraws what a change touches. [`paced`] spaces the changes
//! out and ends the stream after a lifetime; [`Last`] keeps a redraw that
//! changes nothing from sending anything.

use std::time::Duration;

use axum::response::sse::Event;
use futures_util::{Stream, StreamExt as _};
use tokio::time::{Instant, sleep_until};

use crate::Html;

/// How often a live stream redraws, and how long it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    /// The least time between two items. 250 ms by default.
    pub throttle: Duration,
    /// How long the stream runs before the page opens another. A minute by
    /// default.
    pub lifetime: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            throttle: Duration::from_millis(250),
            lifetime: Duration::from_secs(60),
        }
    }
}

/// `changes` at most one per `pace.throttle`, the first at once, until they
/// end or `pace.lifetime` passes.
///
/// A live stream maps each change to the events that redraw what it touched,
/// and [`stream`](crate::stream) sends them:
///
/// ```
/// use askama_datastar::{Html, Last, Pace, paced, stream};
/// use futures_util::StreamExt as _;
/// use tokio::sync::watch;
///
/// fn live(count: watch::Receiver<u64>) -> axum::response::Response {
///     // Every change to the count, starting with the count as it is.
///     let changes = futures_util::stream::unfold((count, true), |(mut count, first)| async move {
///         if !first {
///             count.changed().await.ok()?;
///         }
///         let n = *count.borrow_and_update();
///         Some((n, (count, false)))
///     });
///     let mut last = Last::new();
///     stream(paced(changes, Pace::default()).filter_map(move |n| {
///         let html = Html::trusted(format!(r#"<p id="count">{n}</p>"#));
///         std::future::ready(last.send(&html))
///     }))
/// }
/// ```
///
/// Changes are not merged: a source that gathers what happened since it was
/// last asked, as a `tokio::sync::watch` does, turns a burst inside the
/// throttle into one item by itself.
///
/// Ending on a timer matters: a stream can die without either end noticing,
/// as a suspended phone's does, and the lifetime bounds how long a page sits
/// on one. The page's [`reopen`](crate::reopen) attributes open another
/// within seconds. The lifetime is a deadline: no item comes after it, however
/// the throttle falls or however ready the source is.
pub fn paced<S>(changes: S, pace: Pace) -> impl Stream<Item = S::Item> + Send
where
    S: Stream + Send + 'static,
    S::Item: Send,
{
    let ends = Instant::now() + pace.lifetime;
    futures_util::stream::unfold(
        (Box::pin(changes), None::<Instant>),
        move |(mut changes, last)| async move {
            if let Some(last) = last {
                sleep_until((last + pace.throttle).min(ends)).await;
            }
            if Instant::now() >= ends {
                return None;
            }
            tokio::select! {
                biased;
                () = sleep_until(ends) => None,
                item = changes.next() => item.map(|item| (item, (changes, Some(Instant::now())))),
            }
        },
    )
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
