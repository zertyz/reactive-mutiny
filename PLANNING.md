# Evolutions & History for this project

Here you'll find the *backlogs*, *being-done*, *completed* and *bug reports* for this project.

Issues contain a *prefix* letter and a sequence number, possibly followed by a date and a context-full description


## Prefixes

  - (b) bug fix for broken functionalities
  - (f) new functional requisite
  - (n) new non-functional requisite
  - (r) internal re-engineering / refactor / improvement, to increase speed and/or enable further progress to be done cheaper




# Being-Done

**(f14)** 2023-07-16: Some API changes due to user feedback regarding retries:
  1) No more `try_send*()` familly of functions -- only `send()` & `send_movable()`
  2) Further, `send()` will be renamed to `send_with()` and `send_movable()` will become just `send()`
  3) All the above functions should return data suitable for zero-copy retrying (no longer a simple `bool` or `()`):
     - `Result<(), T>`, being Ok(()) if the sending was successful and `Err(T)` if it was not, so retrying could be attempted without copying
       (notice some types do return a value on success... for these cases, `Result<S,T>` where `S` is the currently returned success value);
     - `Result<(), FnOnce>`, like above, for `send_with(FnOnce)`

    --> It was confirmed those changes are, indeed, zero-cost abstractions: no extra copying is being done and performance is as good as before (when compiling to Release).
  4) Introduce Uni/Multi Channels `is_channel_open()` and, possibly, review the name `keep_streams_running()` / `close_streams()/close_channel()`
  5) Start using the `keen-retry` crate in the `Multi` channels (logging if retrying is needed, retrying in a spin-looping for up to 1 second before giving up)
  6) Due to (5), the `send*()` functions will return `Result<n, Payload>`, where `n` is:
     - *0*  if all listeners received the event in the first shot (without retrying);
     - *>0* if all listeners received the event, but some of them (the returned value), required *retrying*;
     - *<0* if not all listeners were able to receive the event (the abs of the returned value)
  7) Also due to (5), laden the code with `TODO n18` in the places where that followup story should touch/fix.
  8) Take in **(b8)** as part of this story.

**(n18)** 2023-07-27: Incorporate the "Generic Configuration" pattern used by the `reactive-messaging` crate in order to:
  1) Make the arbitrary logic defined in *f14.5* configurable
  2) Move our existing `INSTRUMENTATION` pattern to the `CONFIGURATION` one.

**(b8)** 2023-05-30: No function should panic! for non-bug scenarios: `Result<>`s should be returned instead

**(r2)** 2023-05-30: Complete the socket-server example with fully working server and client sides (with benchmraks)
  1) Simplify / redocument the existing code based on the latest improvements of the reactive-mutiny library
  2) Complete the messaging model, with a minimum working implementation for the server & client sides
  3) Revisit the unit tests, making sure all use cases of the messaging model are covered
  4) Write benchmarks and tune it to perfection (using the appropriate Channels and internal structures)

**(n9)** Include benchmarks for tokio::sync::broadcast -- a Multi channel. If they have good performance, include this channel in our Multi. They also have a "watch" in addition to "broadcast", but it is unknown if watch attends to our requisites




# Backlog
  
**(b4)** 2023-05-30: fix the broken tests for the broken queues after the last abstractions added to it in early May

**(r5)** 2023-05-30: add the github actions for CI/CD

**(r3)** 2023-05-30: Complete the documentation and beautify the code, with attention not to copy & paste any text (use referencing instead)
  1) Remove code duplication, introducing new abstractions when needed
  2) Plot the code diagrams and improve them to their best
  3) Walk through all the examples, making sure all entities are documented in the first level (if references are done in deeper levels, they should be moved to the top levels)
  4) Walk through all code artifacts, making sure nobody is missing any minimal documentation
  5) Provide documentation examples wherever that is applicable

**(f6)** 2023-05-30: add channels for Stacks

**(f7)** 2023-05-30: add the Box allocator


# Done

**(0)** 2022/2023: ((past improvements were not documented here yet))

**(r1)** 2023-05-30: Images used in the README must be .png rather than .svg, so they may appear in crates.io

**(f10)** 2023-06-05: Unis must be able to process all the combinations of fallible/non-fallible & futures/non-futures -- the same for Multi

**(b11)** 2023-06-14: there seems to be no known way of passing the stream processing logic down a complex call chain

**(r12)** 2023-06-14: Unify `UniBuilder` & `Uni`, for types simplification & symmetry with how `Multi`s work -- causing a MAJOR version upgrade




# Unable to reproduce bug reports

**(b13)** 2023-07-15: investigate a possible bug: make sure OgreUnique is not crashing here due to the wrapped type doesn't need dropping (no internal strings in it)... 
                      also, there is the remote possibility of a double-dropping... which should be impossible, but, anyway... do unit tests for those scenarios
                      thread 'tokio-runtime-worker' panicked at 'assertion failed: 0 < pointee_size && pointee_size <= isize::MAX as usize', /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/ptr/const_ptr.rs:694:9
```
  stack backtrace:
    0: rust_begin_unwind
              at /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/std/src/panicking.rs:578:5
    1: core::panicking::panic_fmt
              at /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/panicking.rs:67:14
    2: core::panicking::panic
              at /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/panicking.rs:117:5
    3: core::ptr::const_ptr::<impl *const T>::offset_from
              at /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/ptr/const_ptr.rs:694:9
    4: <reactive_mutiny::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<DataType,ContainerType,_> as reactive_mutiny::ogre_std::ogre_alloc::types::OgreAllocator<DataType>>::id_from_ref
              at /home/luiz/.cargo/registry/src/index.crates.io-6f17d22bba15001f/reactive-mutiny-1.1.2/src/ogre_std/ogre_alloc/ogre_array_pool_allocator.rs:105:13
    5: <reactive_mutiny::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<DataType,ContainerType,_> as reactive_mutiny::ogre_std::ogre_alloc::types::OgreAllocator<DataType>>::dealloc_ref
              at /home/luiz/.cargo/registry/src/index.crates.io-6f17d22bba15001f/reactive-mutiny-1.1.2/src/ogre_std/ogre_alloc/ogre_array_pool_allocator.rs:85:23
    6: <reactive_mutiny::ogre_std::ogre_alloc::ogre_unique::OgreUnique<DataType,OgreAllocatorType> as core::ops::drop::Drop>::drop
              at /home/luiz/.cargo/registry/src/index.crates.io-6f17d22bba15001f/reactive-mutiny-1.1.2/src/ogre_std/ogre_alloc/ogre_unique.rs:129:9
    7: core::ptr::drop_in_place<reactive_mutiny::ogre_std::ogre_alloc::ogre_unique::OgreUnique<reactive_messaging::socket_server::tests::DummyResponsiveClientAndServerMessages,reactive_mutiny::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<reactive_messaging::socket_server::tests::DummyResponsiveClientAndServerMessages,reactive_mutiny::ogre_std::ogre_queues::atomic::atomic_move::AtomicMove<u32,2048_usize>,2048_usize>>>
              at /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/ptr/mod.rs:490:1
    8: reactive_messaging::socket_server::tests::shutdown_process::{{closure}}::{{closure}}::{{closure}}
              at ./src/socket_server.rs:281:17
```