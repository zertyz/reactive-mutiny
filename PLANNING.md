# Foreseable evolutions to this project

This file contains the backlog & implementation plans for all foreseable features to be added / fixes to this project

## Prefixes

  - (b) bug fix for broken functionalities
  - (f) new functional requisite
  - (n) new non-functional requisite
  - (r) internal re-engineering / refactor / improvement, to increase speed and/or enable further progress to be done cheaper

# TODO

  f14) 2023-07-16: Some API changes due to user feedback: 
                     1) `try_send_movable()`, `try_send()` & friends should not return just a bool. They should return:
                          * `Option<T>`, being None if the sending was successful and `Some(T)` if it was not, so retrying could be attempted without copying;
                          * `Option<FnOnce>`, like above, for `try_send(FnOnce)` -- which might be renamed to `try_send_with()`
                          * Introduce `send_movable()` and `send_with()`, receiving an additional callback `on_full: Fn() -> bool`, into which one may add sleep, spin, log, etc.
                        --> we must confirm this API change doesn't copy values when retrying (if compiled for Release).
                     2) Introduce Uni/Multi Channels `is_channel_open()` and, possibly, review the name `keep_streams_running()` / `close_streams()/close_channel()`
  b8) 2023-05-30: No function should panic! for non-bug scenarios: `Result<>`s should be returned instead
  r2) 2023-05-30: Complete the socket-server example with fully working server and client sides (with benchmraks)
    r2.1) Simplify / redocument the existing code based on the latest improvements of the reactive-mutiny library
    r2.2) Complete the messaging model, with a minimum working implementation for the server & client sides
    r2.3) Revisit the unit tests, making sure all use cases of the messaging model are covered
    r2.4) Write benchmarks and tune it to perfection (using the appropriate Channels and internal structures)
  n9) Include benchmarks for tokio::sync::broadcast -- a Multi channel. If they have good performance, include this channel in our Multi. They also have a "watch" in addition to "broadcast", but it is unknown if watch attends to our requisites


# Backlog
  
  b4) 2023-05-30: fix the broken tests for the broken queues after the last abstractions added to it in early May
  r5) 2023-05-30: add the github actions for CI/CD
  r3) 2023-05-30: Complete the documentation and beautify the code, with attention not to copy & paste any text (use referencing instead)
    r3.1) Remove code duplication, introducing new abstractions when needed
    r3.2) Plot the code diagrams and improve them to their best
    r3.3) Walk through all the examples, making sure all entities are documented in the first level (if references are done in deeper levels, they should be moved to the top levels)
    r3.4) Walk through all code artifacts, making sure nobody is missing any minimal documentation
    r3.5) Provide documentation examples wherever that is applicable
  f6) 2023-05-30: add channels for Stacks
  f7) 2023-05-30: add the Box allocator


# Done

  0) 2022/2023: ((past improvements were not documented here yet))
  r1) 2023-05-30: Images used in the README must be .png rather than .svg, so they may appear in crates.io
  f10) 2023-06-05: Unis must be able to process all the combinations of fallible/non-fallible & futures/non-futures -- the same for Multi
  b11) 2023-06-14: there seems to be no known way of passing the stream processing logic down a complex call chain
  r12) 2023-06-14: Unify `UniBuilder` & `Uni`, for types simplification & symmetry with how `Multi`s work -- causing a MAJOR version upgrade


# Unable to reproduce bug reports

  b13) 2023-07-15: investigate a possible bug: make sure OgreUnique is not crashing here due to the wrapped type doesn't need dropping (no internal strings in it)... 
                   also, there is the remote possibility of a double-dropping... which should be impossible, but, anyway... do unit tests for those scenarios
                    thread 'tokio-runtime-worker' panicked at 'assertion failed: 0 < pointee_size && pointee_size <= isize::MAX as usize', /rustc/90c541806f23a127002de5b4038be731ba1458ca/library/core/src/ptr/const_ptr.rs:694:9
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

