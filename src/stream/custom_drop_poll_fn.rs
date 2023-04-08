//! Introduces the `CustomDropPollFn` combinator.\
//! Similar to [futures::stream::poll_fn], but allows running custom code when dropping.\
//! See [custom_drop_poll_fn()] bellow for more.

use core::fmt;
use core::pin::Pin;
use futures::Stream;
use futures::task::{Context, Poll};

/// Stream for the [`poll_fn`] function.
#[must_use = "streams do nothing unless polled"]
pub struct CustomDropPollFn<D: FnOnce(), F> {
    d: Option<D>,
    f: F,
}

impl<D: FnOnce(), F> Unpin for CustomDropPollFn<D, F> {}

impl<D: FnOnce(), F> fmt::Debug for CustomDropPollFn<D, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomDropPollFn").finish()
    }
}

/// Creates a new stream wrapping a function returning `Poll<Option<T>>`.
///
/// Polling the returned stream calls the wrapped function.
///
/// When dropped, the provided custom code runs.
///
pub fn custom_drop_poll_fn<D, T, F>(d: D, f: F) -> CustomDropPollFn<D, F>
    where
        D: FnOnce(),
        F: FnMut(&mut Context<'_>) -> Poll<Option<T>>,
{
    assert_stream::<T, _>(CustomDropPollFn { d: Some(d), f })
}

impl<D, T, F> Stream for CustomDropPollFn<D, F>
    where
        D: FnOnce(),
        F: FnMut(&mut Context<'_>) -> Poll<Option<T>>,
{
    type Item = T;

    #[inline(always)]
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        (&mut self.f)(cx)
    }
}

impl <D: FnOnce(), F> Drop for CustomDropPollFn<D, F> {
    fn drop(&mut self) {
        match self.d.take() {
            None => unreachable!("Bug! Custom Drop FnOnce() code didn't reach the drop() function!!"),
            Some(d) => (d)(),
        }
    }
}
/// From `futures`:
/// Just a helper function to ensure the streams we're returning all have the
/// right implementations.
fn assert_stream<T, S>(stream: S) -> S
    where
        S: Stream<Item = T>,
{
    stream
}