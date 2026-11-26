//! A live stream's loop: everything drawn once, a redraw per change at most
//! every throttle, unchanged regions left out, a drawer's own wake, and an end
//! after the lifetime.

#![allow(clippy::unused_async_trait_impl)]

use std::sync::Arc;
use std::time::Duration;

use askama_datastar::Html;
use askama_datastar::live::{Changes, Draw, Last, Timing, redraws};
use axum::response::sse::Event;
use futures_util::{Stream, StreamExt as _};
use tokio::sync::{Notify, watch};

struct Counted(watch::Receiver<u64>);

impl Changes for Counted {
    type Changed = bool;

    fn everything(&self) -> bool {
        true
    }

    async fn changed(&mut self) -> Option<bool> {
        self.0.changed().await.ok().map(|()| true)
    }
}

/// Whether the count is even: most changes draw the same region.
struct Parity(watch::Receiver<u64>, Last);

impl Draw<bool> for Parity {
    async fn draw(&mut self, changed: &bool) -> Vec<Event> {
        if !changed {
            return Vec::new();
        }
        let parity = Html::trusted(format!("<p id=\"parity\">{}</p>", *self.0.borrow() % 2));
        self.1.send(&parity).into_iter().collect()
    }
}

/// The count, and how many replies have come.
struct Count {
    shown: watch::Receiver<u64>,
    reply: Arc<Notify>,
    replies: u32,
}

impl Draw<bool> for Count {
    async fn draw(&mut self, changed: &bool) -> Vec<Event> {
        let what = if *changed { "count" } else { "reply" };
        let n = *self.shown.borrow();
        vec![
            Html::trusted(format!("<p id=\"count\">{what} {n} {}</p>", self.replies))
                .patch()
                .into(),
        ]
    }

    async fn wake(&mut self) {
        self.reply.notified().await;
        self.replies += 1;
    }
}

fn open(
    count: &watch::Sender<u64>,
    reply: &Arc<Notify>,
) -> std::pin::Pin<Box<dyn Stream<Item = Event> + Send>> {
    let drawn = (
        Parity(count.subscribe(), Last::new()),
        Count {
            shown: count.subscribe(),
            reply: reply.clone(),
            replies: 0,
        },
    );
    let timing = Timing {
        lifetime: Duration::from_secs(5),
        ..Timing::default()
    };
    Box::pin(redraws(Counted(count.subscribe()), drawn, timing))
}

async fn next(events: &mut (impl Stream<Item = Event> + Unpin)) -> String {
    format!("{:?}", events.next().await.expect("an event"))
}

#[tokio::test(start_paused = true)]
async fn a_stream_draws_everything_then_each_change() {
    let (count, _) = watch::channel(0);
    let reply = Arc::new(Notify::new());
    let mut events = open(&count, &reply);
    assert!(next(&mut events).await.contains("parity"));
    assert!(next(&mut events).await.contains("count 0 0"));

    count.send_replace(2);
    let drawn = next(&mut events).await;
    assert!(
        drawn.contains("count 2 0"),
        "the parity is what was last sent, so only the count is: {drawn}"
    );

    count.send_replace(3);
    assert!(next(&mut events).await.contains("parity"));
    assert!(next(&mut events).await.contains("count 3 0"));

    reply.notify_one();
    assert!(next(&mut events).await.contains("reply 3 1"));
}

#[tokio::test(start_paused = true)]
async fn changes_inside_the_throttle_are_one_redraw() {
    let (count, _) = watch::channel(0);
    let mut events = open(&count, &Arc::new(Notify::new()));
    next(&mut events).await;
    next(&mut events).await;
    let start = tokio::time::Instant::now();
    count.send_replace(2);
    assert!(next(&mut events).await.contains("count 2"));
    count.send_replace(4);
    count.send_replace(6);
    assert!(
        next(&mut events).await.contains("count 6"),
        "4 is never drawn"
    );
    assert!(start.elapsed() >= Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn a_stream_ends_after_its_lifetime() {
    let (count, _) = watch::channel(0);
    let mut events = open(&count, &Arc::new(Notify::new()));
    next(&mut events).await;
    next(&mut events).await;
    let start = tokio::time::Instant::now();
    assert!(
        events.next().await.is_none(),
        "nothing changed, so nothing was sent"
    );
    assert_eq!(start.elapsed().as_secs(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_stream_ends_when_its_source_goes() {
    let (count, _) = watch::channel(0);
    let mut events = open(&count, &Arc::new(Notify::new()));
    next(&mut events).await;
    next(&mut events).await;
    drop(count);
    assert!(events.next().await.is_none());
}
