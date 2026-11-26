//! A live stream's pace: the first change at once, then at most one per
//! throttle, ending with its source or at its lifetime, never after it.

use std::time::Duration;

use askama_datastar::{Pace, paced};
use futures_util::{Stream, StreamExt as _};
use tokio::sync::mpsc;
use tokio::time::Instant;

fn source<T: Send + 'static>(rx: mpsc::UnboundedReceiver<T>) -> impl Stream<Item = T> + Send {
    futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|t| (t, rx)) })
}

fn pace() -> Pace {
    Pace {
        lifetime: Duration::from_secs(5),
        ..Pace::default()
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_change_is_at_once_and_the_rest_a_throttle_apart() {
    let (tx, rx) = mpsc::unbounded_channel();
    let start = Instant::now();
    let mut changes = Box::pin(paced(source(rx), pace()));
    for n in 1..=3 {
        tx.send(n).unwrap();
    }
    assert_eq!(changes.next().await, Some(1));
    assert_eq!(start.elapsed(), Duration::ZERO);
    assert_eq!(changes.next().await, Some(2));
    assert_eq!(start.elapsed(), Duration::from_millis(250));
    assert_eq!(changes.next().await, Some(3));
    assert_eq!(start.elapsed(), Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn the_stream_ends_at_its_lifetime() {
    let (_tx, rx) = mpsc::unbounded_channel::<u8>();
    let start = Instant::now();
    let mut changes = Box::pin(paced(source(rx), pace()));
    assert_eq!(changes.next().await, None);
    assert_eq!(start.elapsed(), Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn a_ready_source_and_a_long_throttle_do_not_outlive_the_lifetime() {
    let (tx, rx) = mpsc::unbounded_channel();
    let pace = Pace {
        throttle: Duration::from_secs(10),
        lifetime: Duration::from_secs(15),
    };
    let start = Instant::now();
    let mut changes = Box::pin(paced(source(rx), pace));
    for n in 0..100 {
        tx.send(n).unwrap();
    }
    assert_eq!(changes.next().await, Some(0));
    assert_eq!(changes.next().await, Some(1), "at 10 seconds");
    assert_eq!(
        changes.next().await,
        None,
        "20 seconds would be past the lifetime"
    );
    assert_eq!(start.elapsed(), Duration::from_secs(15));
}

#[tokio::test(start_paused = true)]
async fn the_stream_ends_with_its_source() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut changes = Box::pin(paced(source(rx), pace()));
    tx.send(1).unwrap();
    drop(tx);
    assert_eq!(changes.next().await, Some(1));
    assert_eq!(changes.next().await, None);
}
