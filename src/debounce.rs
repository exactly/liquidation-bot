//! Just emits an event if an minimum amount of time has elapsed.

use tokio::time::{Duration, Instant, Sleep};
use tokio_stream::Stream;

use std::future::Future;
use std::marker::Unpin;
use std::pin::Pin;
use std::task::{self, Poll};

use pin_project_lite::pin_project;

pub(super) fn debounce<T>(duration: Duration, stream: T) -> Debounce<T>
where
    T: Stream,
{
    Debounce {
        delay: tokio::time::sleep_until(Instant::now() + duration),
        duration,
        stream,
        last_item: None,
    }
}

pin_project! {
    /// Stream for the [`debounce`](debounce) function. This object is `!Unpin`. If you need it to
    /// implement `Unpin` you can pin your debounce like this: `Box::pin(your_debounce)`.
    #[must_use = "streams do nothing unless polled"]
    pub struct Debounce<T: Stream> {
        #[pin]
        delay: Sleep,
        duration: Duration,
        // The stream to debounce
        #[pin]
        stream: T,
        last_item: Option<T::Item>,
    }
}

// XXX: are these safe if `T: !Unpin`?
impl<T: Unpin + Stream> Debounce<T> {
    /// Acquires a reference to the underlying stream that this combinator is
    /// pulling from.
    pub fn get_ref(&self) -> &T {
        &self.stream
    }

    /// Acquires a mutable reference to the underlying stream that this combinator
    /// is pulling from.
    ///
    /// Note that care must be taken to avoid tampering with the state of the stream
    /// which may otherwise confuse this combinator.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.stream
    }

    /// Consumes this combinator, returning the underlying stream.
    ///
    /// Note that this may discard intermediate state of this combinator, so care
    /// should be taken to avoid losing resources when this is called.
    pub fn into_inner(self) -> T {
        self.stream
    }
}

impl<T: Stream> Stream for Debounce<T> {
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut me = self.project();
        let dur = *me.duration;
        let item = me.stream.poll_next(cx);
        let mut delay = me.delay.as_mut().poll(cx);
        if let Poll::Ready(value) = item {
            if !is_zero(dur) {
                me.delay.reset(Instant::now() + dur);
                delay = Poll::Pending;
            }
            if value.is_some() {
                *me.last_item = value;
            } else if me.last_item.is_some() {
                return Poll::Pending;
            } else {
                return Poll::Ready(None);
            }
        }
        if let Poll::Pending = delay {
            return Poll::Pending;
        }
        if let None = me.last_item {
            return Poll::Pending;
        }
        Poll::Ready(me.last_item.take())
    }
}

fn is_zero(dur: Duration) -> bool {
    dur == Duration::from_millis(0)
}
