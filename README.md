# reactive-mutiny

*async Event-Driven Reactive Library for Rust with advanced & optimized containers (channels) and Stream executors*

This crate was inspired by the SmallRye's Mutiny library for Java, from which some names were borrowed.
Little had to be done to bring the same functionality to Rust, due to the native functional approach, powerful error
handling, async support and wonderful & flexible Iterator/Stream APIs, so the focus of this work went into bringing the events to
their maximum processing speed & operability: special queues, topics, stacks, channels and Stream executors have been created, offering
a superior performance over the native versions -- inspect the `benchmark` folder for details.

Rust's `reactive-mutiny`, thus, was designed to allow building efficient & elegant asynchronous event processing pipelines (using
Streams -- a.k.a. "async Iterators"), easing microservice architectures, ready for production.

Taste it in this excerpt:

```nocompile
    // on a service:
    pub fn service_message_processor(processor: impl Stream<InMessage>) -> impl Stream<OutMessage> {
        processor.map(|in| {
            // some logic
            OutMessage(format!("Just received '{in}'))
        })
    }
    // on a client:
```

(non compiling nor compeling example above... fix that)

The core of this library is composed of a `Uni` and a `Multi` -- hence the name "Mutiny". Both process streams of events:
  - `Uni` allows **a single listener OR multiple consumers** for each produced payload -- also definable as *allows a single event processing pipeline*;
  - `Multi` allows **multiple listeners AND multiple consumers** for each produced payload, allowing *several event processing pipelines*
    -- or, in Kafka parlance, allows several consumer groups
  - `Multi` may do what `Uni` does, but the former does it faster -- hence, justifying its existence: `Uni` doesn't use any
    reference counting for the payloads and uses a single channel (where `Multi` requires as many as there are listeners).

Moreover, zero-costs metrics & logs are available -- getting optimized away if not used.

# Performance

Here are some performance characteristics gathered from `kickass-app-template`:

# Comparisons

If you're familiar with SmallRye's Mutiny, here are some key differences:
  - Both our `Uni` and `Multi` here process streams of events. On the original library, a Uni is like a single
    "async future" and, since we don't need that in Rust, the names were repurposed: the other Multi is our `Uni` (may also work as our `Multi` when using "subscriptions")
    and the other Uni you may get by just using any Rust's async calls & handling any `Result<>`, for error treatment
  - Each event fed into the pipeline will be executed, regardless if there is an answer at the end; also, there is no "subscription"
    (subscription is achieved by adding pipelines to a `Multi`)
  - Executors & their settings are set when the pair producer/pipeline are created (when the `Uni` / `Multi` object is created): there
    is no .merge() nor .executeAt() to call in the pipeline
  - No Multi/Uni pipeline conversion and the corresponding plethora of functions -- they are simply not needed
  - No Uni retries, as it just doesn't make sense to restrict retries to a particular type. See more at the end of this README.
  - No timeouts are set in the pipeline -- they are a matter for the executor, which will simply cancel events (that are `Future`s) that take longer than the configured executor's maximum
    (SmallRye's Uni timeouts are attainable using Tokio's "futures" timeouts, just like one would do for any async function call)
  - Incredibly faster: Rust's compiler makes your pipelines (and most of this library) behave as a zero-cost abstraction (when compiled in Release mode). Const generics play a great
    role for such optimizations and zero-cost features as well.
  - To fully get the original Mutiny's behavior, you'll have to use:
    - Rust's `reactive-mutiny` (for reactive async event-processing)
    - `Tokio` (to get responses from Futures and to specify timeouts in async calls, async sleeps... saving a ton of APIs for this crate)
    - Streams (the original Mutiny kind of mixes Multi & Stream functionalities -- which, in practice, leads to inefficient abuses of
      the original library's abstractions -- for using a new instance of their Multi where a Stream or Iterator could be used is a common bad parctice / anti-pattern)
    - A general retry mechanism to simulate what SmallRye's Uni have -- but for all `Result<>` types rathar than just for a particular type --
      see it in action in `examples/error-handing-and-retrying` and observe the meaningful & contextful error messages.
