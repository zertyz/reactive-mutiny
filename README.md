# reactive-mutiny

*async Event-Driven Reactive Library for Rust with optimized Stream executors*

This crate was inspired by the SmallRye's Mutiny library for Java, from which some names were borrowed.
Little had to be done to to bring the same functionality to Rust, due to the native functional approach, powerful error
handling, async support and wonderful & flexible Iterator/Stream APIs, so the focus of this work went into bringing the events to
their maximum processing speed & operability: special queues, stacks, channels and Stream executors have been created, offering a superior performance over the native versions -- inspect the `benchmark` folder for details.

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

The core of this library is composed of a `Uni` and a `Multi` -- hence the name "Mutiny". Both processes streams of events:
  - `Uni` allows **a single listener OR multiple consumers** for each produced payload -- also definable as *allows a single event processing pipeline*;
  - `Multi` allows **multiple listeners AND multiple consumers** for each produced payload, allowing *several event processing pipelines* -- or, in Kafka parlance, allows several consumer groups
  - `Multi` may do what `Uni` does, but the former does it faster -- hence, justifying its existence: `Uni` don't use any
    reference counting for the payloads and use a single channel (where `Multi` requires as many as there are listeners).

Moreover, zero-costs metrics & debugging are available -- getting optimized away if not used.

If you're familiar with SmallRye's Mutiny, here are some key differences:
  - Both our `Uni` and `Multi` here process streams of events. On the original library, a Uni is like a single
    "async future" and, since we don't need that in Rust, the names were repurposed: the other Multi is our `Uni` and the
    other Uni you may get by just using any Rust's async calls & handling any `Result<>`, for error treatment
  - Each event fed into the pipeline will be executed, regardless if there is an answer at the end; also, there is no "subscription"
    (subscription is achieved by adding pipelines to a `Multi`)
  - Executors & their settings are set when the pair producer/pipeline are created (when the `Uni` / `Multi` object is created): there
    is no .merge() nor .executeAt() to call in the pipeline
  - No Multi/Uni pipeline conversion and the corresponding plethora of functions -- they are simply not needed
  - No timeouts are set in the pipeline -- they are a matter for the executor, which will simply cancel events (that are `Future`s) that takes longer than the configured executor's maximum
    (SmallRye's Uni timeouts are attainable using Tokio's "futures" timeouts, just like one would do for any async function call)
  - Incredibly faster: Rust's compiler makes your pipelines (and most of this library) behave as a zero-cost abstraction (when compiled in Release mode). Const generics play a great hole for such optimizations and zero-cost features as well.
  - To fully get the original Mutiny's behavior, you'll have to use:
    - Rust's `reactive-mutiny` (for reactive async event-processing)
    - `Tokio` (to get responses from Futures and to specify timeouts in async calls, async sleeps... saving a ton of APIs)
    - Streams (the original Mutiny kind of mixes Multi & Stream functionalities -- which, in practice, leads to inefficient abuses of
      the original library's abstractions -- for using a new instance of their Multi where a Stream or Iterator could be used is a common bad parctice / anti-pattern)
