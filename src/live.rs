//! Live streams that keep a page current.
//!
//! Every stream has one shape, [`Live`]: it opens from the page's signals,
//! draws the regions a change touches, and may wait on a change of its own,
//! such as a reply it asked for. What changes across the application, and the
//! regions every page shows (a status line, the navigation), are the
//! application's [`Frame`], written once. [`serve`] runs any stream:
//!
//! - It draws everything once, then waits on the frame's changes or the
//!   stream's own, and draws again. Nothing is polled.
//! - It draws at most every [`Frame::THROTTLE`], so a burst of changes is one
//!   redraw. A change source that reports what changed since it was last
//!   asked, as a `tokio::sync::watch` does, gathers a burst by itself.
//! - It ends after [`Frame::LIFETIME`]. The page's [`reopen`](crate::reopen)
//!   attributes open it again within seconds, as they do a stream that ends
//!   or goes quiet for any other reason. A stream can die without either end
//!   noticing (a suspended phone, a replaced server), and ending it on a timer
//!   bounds how long a page sits on one.
//!
//! A region is sent only when it differs from what was last sent, with
//! [`Last`], so a redraw that changes nothing sends nothing.
//!
//! A stream never follows navigation itself: it draws the page its signals
//! describe, and the page opens it afresh when they change
//! ([`reopen_on`](crate::reopen_on), as [`page::Whole::live`](crate::page::Whole::live)
//! writes it).

use std::future::Future;
use std::time::Duration;

use axum::extract::State;
use axum::response::Response;
use axum::response::sse::Event;
use futures_util::StreamExt as _;
use serde::de::DeserializeOwned;
use tokio::time::{Instant, sleep, sleep_until};

use crate::{Html, Signals, stream};

/// What every live stream of an application shares: where changes come from,
/// and the regions every page shows.
///
/// One is opened for each stream, so it holds that stream's subscriptions and
/// what it last sent of the shared regions.
pub trait Frame: Sized + Send + 'static {
    /// The application's state, as axum's router holds it.
    type State: Clone + Send + Sync + 'static;
    /// What a stream tells the frame about the page it serves, such as the
    /// entry of the navigation to mark. `()` when nothing.
    type Place: Sync + 'static;
    /// What changed since the last redraw. Its default is that nothing did,
    /// which is what a stream's own [`Live::wake`] draws with.
    type Changed: Default + Send + Sync;

    /// The least time between two redraws.
    const THROTTLE: Duration = Duration::from_millis(250);
    /// How long a stream runs before the page opens another.
    const LIFETIME: Duration = Duration::from_secs(60);

    /// Subscribed to the application's changes, for a stream serving `place`.
    fn open(state: &Self::State, place: &Self::Place) -> impl Future<Output = Self> + Send;

    /// What the first draw is told: everything changed.
    fn everything() -> Self::Changed;

    /// The next change, or `None` when the source has gone, which ends the
    /// stream.
    fn changed(&mut self) -> impl Future<Output = Option<Self::Changed>> + Send;

    /// The regions every page shows that `changed` touches, before the
    /// stream's own. None, unless the layout has live regions of its own.
    fn draw(
        &mut self,
        state: &Self::State,
        changed: &Self::Changed,
    ) -> impl Future<Output = Vec<Event>> + Send {
        let _ = (state, changed);
        async { Vec::new() }
    }
}

/// A page's live stream.
///
/// ```
/// # use askama_datastar::Html;
/// # use askama_datastar::live::{Frame, Last, Live};
/// # use axum::response::sse::Event;
/// # #[derive(Clone)] struct App { jobs: tokio::sync::watch::Receiver<u64> }
/// # struct Changes(tokio::sync::watch::Receiver<u64>);
/// # impl Frame for Changes {
/// #     type State = App; type Place = (); type Changed = bool;
/// #     async fn open(app: &App, (): &()) -> Self { Self(app.jobs.clone()) }
/// #     fn everything() -> bool { true }
/// #     async fn changed(&mut self) -> Option<bool> { self.0.changed().await.ok().map(|()| true) }
/// # }
/// #[derive(serde::Deserialize, Default)]
/// #[serde(default)]
/// struct Shown {
///     url: String,
/// }
///
/// struct Jobs {
///     list: Last,
/// }
///
/// impl Live for Jobs {
///     type Frame = Changes;
///     const PLACE: () = ();
///     type Signals = Shown;
///
///     async fn open(_: &App, _: Shown) -> Self {
///         Self { list: Last::new() }
///     }
///
///     async fn draw(&mut self, app: &App, changed: &bool) -> Vec<Event> {
///         if !changed {
///             return Vec::new();
///         }
///         let n = *app.jobs.borrow();
///         let list = Html::trusted(format!(r#"<p id="jobs">{n} jobs</p>"#));
///         self.list.send(&list).into_iter().collect()
///     }
/// }
/// ```
pub trait Live: Sized + Send + 'static {
    /// The application's frame.
    type Frame: Frame;
    /// What the stream tells the frame about the page it serves.
    const PLACE: <Self::Frame as Frame>::Place;
    /// The page's signals the stream opens from, read leniently as
    /// [`Signals`] reads them. The address is among them, as
    /// [`page::URL`](crate::page::URL).
    type Signals: DeserializeOwned + Default + Send;

    /// Ready to draw the page `signals` describe.
    fn open(
        state: &<Self::Frame as Frame>::State,
        signals: Self::Signals,
    ) -> impl Future<Output = Self> + Send;

    /// The regions `changed` touches, each sent if it differs from what was
    /// last sent. Everything, the first time.
    fn draw(
        &mut self,
        state: &<Self::Frame as Frame>::State,
        changed: &<Self::Frame as Frame>::Changed,
    ) -> impl Future<Output = Vec<Event>> + Send;

    /// Resolves when the stream has something of its own to draw, such as a
    /// peer's reply. It must be safe to drop before it resolves, as
    /// `tokio::select!` needs. Never, unless the stream says.
    fn wake(&mut self) -> impl Future<Output = ()> + Send {
        std::future::pending()
    }
}

/// Runs stream `L` for the page whose signals the request carries, as a
/// handler: `get(live::serve::<Jobs>)`.
pub async fn serve<L: Live>(
    State(state): State<<L::Frame as Frame>::State>,
    Signals(signals): Signals<L::Signals>,
) -> Response {
    let run = Run::<L> {
        frame: L::Frame::open(&state, &L::PLACE).await,
        page: L::open(&state, signals).await,
        state,
        ends: Instant::now() + L::Frame::LIFETIME,
        drawn: false,
    };
    stream(futures_util::stream::unfold(run, Run::next).flat_map(futures_util::stream::iter))
}

/// A running stream, between two redraws.
struct Run<L: Live> {
    frame: L::Frame,
    page: L,
    state: <L::Frame as Frame>::State,
    ends: Instant,
    drawn: bool,
}

impl<L: Live> Run<L> {
    /// The next redraw's events, once there is a change to draw; `None` once
    /// the stream is over.
    async fn next(mut self) -> Option<(Vec<Event>, Self)> {
        let changed = if self.drawn {
            sleep(<L::Frame as Frame>::THROTTLE).await;
            tokio::select! {
                changed = self.frame.changed() => changed?,
                () = self.page.wake() => Default::default(),
                () = sleep_until(self.ends) => return None,
            }
        } else {
            self.drawn = true;
            L::Frame::everything()
        };
        let mut events = self.frame.draw(&self.state, &changed).await;
        events.extend(self.page.draw(&self.state, &changed).await);
        Some((events, self))
    }
}

/// A region as last sent, to send it again only when it differs.
///
/// A region that renders something new on every draw from the same state,
/// such as a fresh random name, is compared by what stays put:
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
