# Evolutions & History for this project

Here you'll find the *backlogs*, *being-done*, *completed* and *bug reports* for this project.

Issues contain a *prefix* letter and a sequence number, possibly followed by a date and a context-full description


## Prefixes

  - (b) bug fix for broken functionalities
  - (f) new functional requisite
  - (n) new non-functional requisite
  - (r) internal re-engineering / refactor / improvement, to increase speed and/or enable further progress to be done cheaper




# Being-Done

**(f21)** 2024-03-02: Possibly introduce the `OgreBoundedBoxAllocator` to enable advanced usage in `reactive-messaging`:
This will enable `reactive-messaging` to receive variable sized binary messages.
This should be better explored, as it seems there are options. Use cases tests should be added for sending
RKYV's "Archived" structs and changes / implementations should follow from there.

**(f22)** 2024-09-01: Complete & homogenize all channel implementations:
Specially after **(f20)**, channels got new experimental & cool functionalities -- even if not all channels got an initial
implementation for the features introduced there. This story is about making sure channels properly implement all the features
they support & that a final pass was made to make all of them use similar patterns and, preferably, the same traits.
  * **New Traits**: The new traits would be: `AsyncChannel`, `AdvancedChannelSemantics` & possibly others. Those are
    "Functional Traits", as they specify additional operations the channel support. Keep in mind that not all channels may implement
    all functionalities efficiently. As an example, the Crossbeam channel would need an auxiliary vector to support the
    `reserve_slot()` / `try_send_reserved()` / `try_cancel_slot_reserve()` semantics -- and, since our crate is to provide
    as-efficient-as-possible channels, this kind of workaround should not be done.
  * Multi (arc) channels bug: The issue reported at https://github.com/zertyz/reactive-mutiny/issues/1 can also make part
    of this review and, should we choose not to drop those channels, the inability to support the use case described there
    should be expressed by the traits the affected channels implement or not.
  * Take an extra care to complete the implementation of the `mmap_log` channel, also adding traits that expresses its
    unique abilities.
  * Some trait patterns, coded here in base64, may serve as an inspiration:
    dHJhaXQgQ2hhbm5lbCB7CiAgICBmbiBzZW5kKCZzZWxmKTsKfQoKdHJhaXQgQWR2YW5jZWRDaGFu
    bmVsU2VtYW50aWNzIHsKICAgIGZuIHJlc2VydmUoJnNlbGYpOwp9Cgp0cmFpdCBBc3luY0NoYW5u
    ZWw6IENoYW5uZWwgKyBBZHZhbmNlZENoYW5uZWxTZW1hbnRpY3MgewogICAgZm4gc2VuZF93aXRo
    X2FzeW5jKCZzZWxmKSB7CiAgICAgICAgcHJpbnRsbiEoIlRoaXMgY2hhbm5lbCBhdXRvbWF0aWNh
    bGx5IGltcGxlbWVudHMgYEFzeW5jQ2hhbm5lbGAiKTsKICAgIH0KfQoKLy8gTm8gYmxhbmtldCBp
    bXBsZW1lbnRhdGlvbiBuZWVkZWQgaGVyZQoKLy8gVXNhZ2UKCnN0cnVjdCBGdWxsU3luYzsKaW1w
    bCBDaGFubmVsIGZvciBGdWxsU3luYyB7CiAgICBmbiBzZW5kKCZzZWxmKSB7CiAgICAgICAgcHJp
    bnRsbiEoIkZ1bGxTeW5jIGltcGxlbWVudHMgQ2hhbm5lbCIpOwogICAgfQp9CgpzdHJ1Y3QgQXRv
    bWljOwppbXBsIENoYW5uZWwgZm9yIEF0b21pYyB7CiAgICBmbiBzZW5kKCZzZWxmKSB7CiAgICAg
    ICAgcHJpbnRsbiEoIkF0b21pYyBpbXBsZW1lbnRzIENoYW5uZWwiKTsKICAgIH0KfQoKaW1wbCBB
    ZHZhbmNlZENoYW5uZWxTZW1hbnRpY3MgZm9yIEF0b21pYyB7CiAgICBmbiByZXNlcnZlKCZzZWxm
    KSB7CiAgICAgICAgcHJpbnRsbiEoIkF0b21pYyBpbXBsZW1lbnRzIEFkdmFuY2VkQ2hhbm5lbFNl
    bWFudGljcyIpOwogICAgfQp9CgovLyBPdmVycmlkZSB0aGUgZGVmYXVsdCBpbXBsZW1lbnRhdGlv
    biBmb3IgQXN5bmNDaGFubmVsIGZvciBBdG9taWMKaW1wbCBBc3luY0NoYW5uZWwgZm9yIEF0b21p
    YyB7CiAgICBmbiBzZW5kX3dpdGhfYXN5bmMoJnNlbGYpIHsKICAgICAgICBwcmludGxuISgiQXRv
    bWljIGNoYW5uZWwgYWxzbyBpbXBsZW1lbnRzIGBBc3luY0NoYW5uZWxgIC0tIG92ZXJyaWRpbmcg
    dGhlIGRlZmF1bHQgaW1wbGVtZW50YXRpb24iKTsKICAgIH0KfQoKZm4gbWFpbigpIHsKICAgIGxl
    dCBzMSA9IEZ1bGxTeW5jOwogICAgczEuc2VuZCgpOwogICAgLy8gczEuc2VuZF93aXRoX2FzeW5j
    KCk7IC8vIENvbXBpbGUgZXJyb3IsIEZ1bGxTeW5jIGRvZXMgbm90IGltcGxlbWVudCBBZHZhbmNl
    ZENoYW5uZWxTZW1hbnRpY3Mgb3IgQXN5bmNDaGFubmVsCgogICAgbGV0IHMyID0gQXRvbWljOwog
    ICAgczIuc2VuZCgpOwogICAgczIucmVzZXJ2ZSgpOwogICAgczIuc2VuZF93aXRoX2FzeW5jKCk7
    IC8vIFdvcmtzIGJlY2F1c2UgQXRvbWljIGltcGxlbWVudHMgYm90aCBDaGFubmVsIGFuZCBBZHZh
    bmNlZENoYW5uZWxTZW1hbnRpY3MKfQoK


# Backlog
  
**(r18)** 2023-08-02: Eventually drop all Multi Arc channels, as they are a not good fit for our retrying model -- see the TODOs with the same date 

**(b8)** 2023-05-30: No function should panic! for non-bug scenarios: `Result<>`s should be returned instead

**(r2)** 2023-05-30: Complete the socket-server example with fully working server and client sides (with benchmraks)
  1) Simplify / redocument the existing code based on the latest improvements of the reactive-mutiny library
  2) Complete the messaging model, with a minimum working implementation for the server & client sides
  3) Revisit the unit tests, making sure all use cases of the messaging model are covered
  4) Write benchmarks and tune it to perfection (using the appropriate Channels and internal structures)

**(n9)** Include benchmarks for tokio::sync::broadcast -- a Multi channel. If they have good performance, include this channel in our Multi. They also have a "watch" in addition to "broadcast", but it is unknown if watch attends to our requisites

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

**(f20)** 2024-03-02: Expose `BoundedOgreAllocator` functionalities to the `ChannelProducer`:
Some users of our trait -- specially `reactive-messaging` -- work in close relation with `ChannelProducer`s.
To allow them to have a simpler type system, we should add more items to our trait to intermediate the talk with the
underlying allocator:
  1) Allocate a slot
  2) Free an allocated slot
  3) Upgrade the medium version, as this API change introduces backwards incompatibilities
  4) Bonus: The above lets us easily add a `.send_with_async(async_producer_closure)` method. Lets do it!

**(r19)** 2024-03-02: Flexibilize our `OgreAllocator`:
Currently, our `OgreAllocator` is better described as a `BoundedOgreAllocator` -- due to the fact that allocated slots have an `id`.
Although we are Generic on the `SlotType`, the returned product of the allocation operations is fixed as `&mut SlotType`.
and, more importantly, there is no way for an allocation to be done externally -- as required in **f(21)**.
This task is about improving this situation to enabling **(f20)** and **(f21)** along the way. Steps:
  1) Introduce the GAT type `OwnedSlotType`. For reference, all current usages of this trait
     will have it set to `SlotType` -- a different usage will be `Box<SlotType` when doing **f(21)**
  2) Add the **f(21)** related function `with_external_alloc()`, enabling an allocation to be done externally, via a callback.
     Note that implementing this method might not make sense to the current Array Allocator... maybe a future refactor may solve this better. 
  3) Add the async version of `alloc_with()`, also related to **f(21)**
  4) Rename the trait to `BoundedOgreAllocator`.

**(f14)** 2023-08-02: Some API changes due to user feedback regarding retries:
  1) All the Channel publishing functions (all the way down to the containers) are now fully zero-copy compliant -- including any retrying.
     For this, they all return back the payload if pushing the data into the container failed
     --> It was confirmed those changes are, indeed, zero-cost abstractions: no extra copying is being done and performance is as good as before (when compiling to Release).
  2) Rethinking the API: the functions now are `send_with()`, `send()` & `send_derived()`
  3) The aforementioned functions returns a `keen-retry` result, enabling users to add their own retry logic 
  4) Introduced Uni/Multi Channels `is_channel_open()` to provide information for retrying logic
  5) **(b8)** can also be considered done, as panic! is no longer part of any (non-bug) logic

**(r12)** 2023-06-14: Unify `UniBuilder` & `Uni`, for types simplification & symmetry with how `Multi`s work -- causing a MAJOR version upgrade

**(b11)** 2023-06-14: there seems to be no known way of passing the stream processing logic down a complex call chain

**(f10)** 2023-06-05: Unis must be able to process all the combinations of fallible/non-fallible & futures/non-futures -- the same for Multi

**(r1)** 2023-05-30: Images used in the README must be .png rather than .svg, so they may appear in crates.io

**(0)** 2022/2023: ((past improvements were not documented here yet))



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


# Discussions For Future Decisions

## Mmap Logging

### QUESTION
I need to explore some database alternatives for my Rust application.

It is being done in Rust and the data model may be designed to be over the network or locally, to be synchrinized later, after the "hot path hours are done".

I'd first consider a centralized database for big data, as I would have to deal with billions of rows per day.

I want to explore the sustained throughput achieved by using a single thread / single connection.

Considering a modern server hardware, could Cassandra or ScillaDB, by any means, either of these reach 8 million rows per second insertions for a 256 sized record with just a unique ID and no partitioning? Partitioning would be later included, but I want to take it out of the equation (assuming it would slow down inserts) as I want to explore also other non distributed databases).

Are there any other known network databases capable of such feat on modern hardware -- again, in a single thread on the client software driving the inserts through a single connection?

The single thread / single connection is important so I can level up the latency performance, addressing scalability later to achieve better total throughput.

If not a network database, what if we include embedded databases?

We could also include Redis (either network or embedded, if possible) and LMDB and similar things. I would prefer a pure Rust crate for that, but I believe there are none?

Anyway, could these figures be achieved for disk based storage?

As a baseline, considering such records would be inserted into plain text file (append basis), how many million records per second would be expected -- in a single thread -- by modern hardware?

### ANSWER

Below is a broad analysis based on existing benchmarks, known architectural limits, and realistic single-thread performance constraints. I’ll start with the “headline” question—8 million inserts per second, each record ~256 bytes, from a single client thread—and then address various database types (Cassandra, Scylla, Redis, LMDB, etc.) and plain file I/O as a baseline.
1. The 8-million-inserts-per-second target

   Data volume:
   8 million records/sec × 256 bytes = 2,048 MB/s (i.e. ~2 GB/s) of payload.
   To achieve 8M inserts/sec, your single thread has essentially 125 ns to process each record (1 second / 8,000,000).

   Where the bottlenecks usually appear:
   Network: At 2 GB/s, that’s ~16 Gb/s, which requires a 25 Gbps or 40 Gbps link (accounting for overhead). Single-thread throughput at that scale is almost always limited by high network latency or the overhead of single-thread TCP calls, or by the DB server’s need to ingest and index data.
   Syscall overhead: Even if the network is extremely fast or if you’re writing to local disk, sustaining 8 million tiny operations in a single thread is tough. Many systems rely on parallelism or pipelined/batched I/O to reach high total throughput.
   Server-side ingestion: If the database must do any kind of parsing, indexing, partitioning, or journaling, that also takes CPU cycles. With one client thread and one connection, you lose the advantage of concurrency on the DB side.

Bottom line: For network-based databases on modern hardware, it’s very unlikely you’ll sustain 8M inserts/s on a single thread/connection in a realistic environment. Some specialized solutions or custom in-memory pipelines might approach those rates, but typical off-the-shelf distributed stores (Cassandra/Scylla) or simpler in-memory DBs (Redis) don’t generally publish single-thread benchmarks near 8M ops/s at 256 bytes each.
2. Cassandra or ScyllaDB specifically

   Cassandra: Generally scales horizontally but uses multiple connections/threads to achieve high ingest. Single-thread ingestion throughput is relatively modest because of the overhead of replication, partitioning logic, and the Java-based stack.
   ScyllaDB: Often shows higher throughput than Cassandra due to its shard-per-core design in C++. However, it also expects concurrency from multiple connections and multiple shards to shine. Even if a single shard can go fast, it typically does so with concurrency rather than literal single-thread synchronous inserts.
   Partitioning: In practice, partitioning can help with ingestion speed at scale because writes end up being handled in parallel by multiple threads/nodes/shards. If you truly funnel everything into one logical partition with one connection, you lose out on that parallelism.

Realistically, neither Cassandra nor Scylla is known to achieve 8 million rows/s from a single-thread client link in a truly durable/committed sense. Their big strength is horizontal concurrency and parallelism across many connections.
3. Other networked databases

   Redis:
   Single-threaded server by default (though there are threads for I/O). You can achieve high ops/s with pipelining, but typically for smaller data or a large cluster.
   8M ops/s with 256 bytes each from a single client thread is also not typical. Redis can approach millions of simple GET/SET ops/s with pipelining and concurrency, but pushing 2 GB/s just from a single thread is a tall order.
   Special HPC or in-memory key-value stores: If you had a specialized in-memory store with RDMA-based networking and a data layout that needs minimal overhead, you might get close to multiple million ops/s. But these solutions are often custom/academic or require very specialized hardware and concurrency to keep the pipeline full.

In short, for general-purpose open-source network databases, there is no commonly known solution that can sustain 8M writes/s at 256 bytes each on a single connection/thread.
4. Embedded databases

When you bring the data store into the same process (embedded), you remove network overhead and can sometimes get much higher throughput with the right hardware:

    LMDB: Extremely fast for read-heavy or for append-only patterns on fast storage. It can exceed millions of transactions/second if all data fits in memory and you do minimal overhead. But the concurrency model is usually a single-writer, multiple-readers pattern. Achieving 8 million distinct insertions per second might still be limited by how quickly you can commit pages to disk (or to the OS page cache).
    RocksDB: Also quite performant. Achieving several million write ops/s across many threads is doable on fast SSDs or Optane, but again, usually that’s with concurrency. With a single thread, you might see 1–2 million writes/s in extremely optimized scenarios, perhaps less in typical scenarios.
    Pure-Rust embedded: There aren’t many highly tuned pure-Rust embedded DBs for heavy ingestion on the scale of millions of writes/s. Most production-grade embedded key-value engines (e.g., RocksDB, LMDB, WiredTiger) are in C/C++. There are some Rust wrappers, but not a well-known pure-Rust engine that’s on par with LMDB or RocksDB performance at these extreme insertion rates.

So while embedded DBs can get closer to that throughput than a distributed network database (thanks to removing the network round-trips), even then 8 million inserts/s on a single CPU thread is extremely optimistic for general-purpose libraries that maintain consistency on disk.
5. Plain text file (append-only) as a baseline

   A purely sequential file append is the absolute simplest approach: open a file, write(…), keep going. Modern NVMe SSDs can sustain 3+ GB/s in sequential writes, so theoretically you could push 2–3 GB/s from a single thread if your OS buffer cache, I/O scheduling, and CPU overhead for system calls aren’t the bottleneck.
   At 2 GB/s, you’d be near 8 million × 256 bytes/s. In the absolute best circumstances, you might get that raw speed. But realistically:
   System call overhead: A single write() per record is too expensive (8 million writes/s means 8 million system calls/s). You’d need to buffer and do large block writes to reduce syscalls.
   Additional overhead for newlines, minimal formatting, or anything else.
   Realistic file systems can have fragmentation or journaling overhead.

Nonetheless, “multi-million lines per second” is definitely more feasible if you do large buffered writes to an append-only file in a single thread on a modern NVMe drive. You’re basically limited by (a) memory copy speed, (b) system call overhead, and (c) the raw device bandwidth. With a well-tuned approach, you might get close to a couple million lines per second. Hitting 8 million is pushing the envelope, but not absolutely impossible if your entire path is extremely optimized (e.g., huge pwrite calls, minimal overhead, large I/O buffers, direct I/O, etc.).
6. Summary and feasibility check

   Network, single-thread: Achieving 8M inserts/s of 256-byte records implies ~2 GB/s plus overhead. Virtually no standard distributed DB will do that on a single connection, for both hardware and software reasons (latency, indexing overhead, concurrency design).
   Embedded: You’ll remove the network overhead but still face the raw overhead of persisting 8 million new records per second. Even high-performance libraries rarely demonstrate that from a single thread, especially if they’re doing indexing or any ACID compliance.
   Plain file: The simplest approach might let you come closest, if all you do is write a continuous data stream in large buffered blocks. You might see 2–3 GB/s on a very fast SSD or RAM disk. The limiting factor could be system calls or OS overhead. So the “single-thread, 8M rec/s” bar is maybe reachable if you only do large streaming writes and no indexing.

In other words:

    Cassandra/Scylla: No, not at that single-thread rate.
    Other network DB: Same as above, generally no.
    Redis: Probably no, not at that single-thread rate. Possibly with client pipelining you’d get into the 1–2 million ops/s for trivial operations in memory, but 8 million at 256 bytes each is too high in typical setups.
    Embedded DB: More feasible than network, but 8 million is still extremely high. Achieving even a couple of million writes/s single-thread is already pushing it.
    Pure Rust: Not at that scale. There isn’t a well-known pure-Rust DB that can do multi-million durable writes/s in a single thread.
    Plain text: You can come somewhat close if you buffer large writes and you have a top-of-the-line SSD or RAID array giving well over 2 GB/s of sequential throughput. Even then, 8 million lines/s is near the edge.

Hence, for a realistic disk-based or network-based database with normal durability or consistency guarantees, 8 million single-thread inserts per second for 256-byte records is beyond what is commonly seen. If your design absolutely requires that raw ingestion speed from one thread, you’d likely need to:

    Simplify to a near-bare-metal streaming approach (like a log-structured file).
    Possibly keep the data purely in-memory with a specialized pipeline, only flushing asynchronously to disk.
    Rely on concurrency (multiple threads/connections) to aggregate the performance.

Final Takeaway

    Single-thread, single-connection at 8M 256-byte records/s is beyond the typical capabilities of mainstream DB systems (Cassandra, Scylla, Redis) and even top-tier embedded engines, if they’re doing real indexing or disk persistence.
    The closest you might get is a purely sequential append to a fast disk (2–3 GB/s), but that would require carefully batched writes and minimal overhead.
    In practice, you’d use concurrency to reach that scale, not a single-thread approach.

### QUESTION

Going back to the simplest plain text file approach: what if we are using mmap?

This would limit reduce system calls to a minimum and could allow for great latencies, right?

More: while the application could do inserts in a single thread, the OS might use other threads of their own to actually write the data, disrupting the userland writer to a minimum, considering a multi-cpu server.

Please comment on that.

### ANSWER

Using mmap for a high-throughput, append-like logging scenario can certainly reduce the per-write syscall overhead (since you’re just doing memory stores rather than calling write repeatedly). In principle, this can help a single-threaded application achieve higher throughput, because:

    Fewer system calls: Instead of issuing a write() syscall for each block of data, you’re simply writing to memory. The OS will eventually flush those pages to disk (via the page cache and background writeback threads).
    Asynchronous OS writeback: The kernel can push out dirty pages on multiple CPU cores while your single user thread just keeps writing. This background flushing can overlap with your user-space activity, effectively parallelizing I/O behind the scenes.

However, there are still some practical considerations and potential bottlenecks:
1. Page fault overhead and pre-allocation

   If you mmap a large file (or use something like MAP_NORESERVE or lazy allocation), you might incur page faults as you write to pages for the first time. That can create overhead in a tight loop of 8 million writes per second.
   One way to mitigate is to use MAP_POPULATE or explicitly “touch” the pages in advance so they’re faulted in before your tight insertion loop starts. Or preallocate the file on disk to ensure you can map it fully.

2. Page cache pressure

   At 2 GB/s, you’ll generate a lot of dirty pages very quickly. The OS’s page cache and writeback mechanisms have to keep up, or else your process might get throttled when the dirty ratio threshold is reached.
   In a best-case scenario on modern SSD or NVMe hardware with enough concurrency in the kernel’s block layer, you might maintain that throughput if the device is physically capable of ~2 GB/s sequential writes. Otherwise, you’ll eventually stall or see performance degrade.

3. Memory bandwidth

   Writing 8 million 256-byte records per second is ~2 GB/s. Modern servers can handle tens of GB/s in memory bandwidth, so purely from a CPU–memory perspective, 2 GB/s is often feasible. The question is whether the memory subsystem plus the kernel’s background I/O can keep the pipeline flowing.

4. Latency vs. throughput

   From the user thread’s perspective, mmap writes can be very low-latency (just memory store instructions). However, if you need durability at each insert, you’d have to call msync(MS_SYNC) or use fsync after each chunk. That kills throughput because you’re forcing a synchronous flush.
   If your use case tolerates eventual persistence (i.e., it’s okay if the OS flushes the data later), then mmap plus background writeback can indeed give good perceived latency and high throughput in user space.

5. Data integrity

   Similar to the above point: if your application can handle data loss on power failure (i.e., you only rely on the OS to eventually flush), that’s fine. But if you need guaranteed, on-disk durability, you’ll need some kind of forced flush. That reduces throughput drastically—whether you’re using write() or mmap.

6. File expansion

   If the file needs to grow beyond the initially mmap-ed region, you’d have to remap or manage the segments. That’s an extra layer of complexity if the log is unbounded. One workaround is to allocate a large file up front (if feasible) to avoid frequent re-mappings.

Could mmap achieve higher throughput than traditional writes?

Yes, especially if you:

    Use a single thread but avoid synchronous flushes on each record.
    Use large preallocated files and pre-fault the pages.
    Rely on the OS to flush data in the background to a high-performance device.

With these conditions, you can indeed reduce the overhead to near the raw speed of copying memory around, deferring the real I/O cost to the kernel. That can let you push millions of inserts per second in user space.

But you are still subject to:

    The limits of disk bandwidth (or the I/O subsystem).
    Potential OS caching throttle if you outpace disk writes for too long.
    The need to eventually persist data if you require reliability.

Bottom line

    Yes, using mmap can help reduce system call overhead and can leverage kernel concurrency for flushing. That often gives you better single-thread throughput than repeated write() calls.
    However, if your ultimate storage device doesn’t sustain 2+ GB/s on average (or if the OS’s dirty page limits force you to block), you won’t reach that mythical “8 million records/s” level in steady state.
    Latency to merely write data in user space can be very low, but if you need guaranteed on-disk durability at each insert, you’ll have to sync frequently, which kills throughput.

So, mmap is an excellent option to minimize userland overhead for high-speed appends—just be sure your hardware and OS caching policies can handle the rate at which you generate dirty pages.


### CONCLUSIONs

From the above discussions with ChatGPT o1 model made in 2025-03-01, it seems the following benchmarks
for the `MmapLog` is among the best ones possible, even considering theorethical achievements.

The following "8 million rows per second insertions" were the best mmap performance achieved by running our benchmarks on a modest Dell Latitude for 10 times in a row: 

```
[luiz@DellLap reactive-mutiny]$ clear; Ps="firedragon chrome waybar nm-applet"; for p in $Ps; do pkill -f -stop $p; done;  rm /tmp/*.mmap; sudo sync; sleep 10; ./target/release/examples/all-channels;   for p in $Ps; do pkill -f -cont $p; done
On this code, you may see how to build `Uni`s and `Multi`s using all the available channels
-- each providing tradeoffs between features and performance.
So, here are performance characteristics of passing our `ExchangeEvent` through all the different channels -- for all `Uni`s and `Multi`s:

Uni:
    Move:
        Atomic    : 14394614.03/s ✓
        Crossbeam : 20434269.42/s ✓
        Full Sync : 25295941.94/s ✓
    Zero-Copy:
        Atomic    : 8361108.92/s ✓
        Full Sync : 19131507.02/s ✓

Multi:
    Arc:
        Atomic    : 2920370.90/s ✓
        Crossbeam : 3700106.71/s ✓
        FullSync  : 2640500.79/s ✓
    OgreArc:
        Atomic    : 2721195.12/s ✓
        FullSync  : 2638375.31/s ✓
    Reference:
        MmapLog   : 7977614.59/s ✓
```

Next steps should be planned to advance with the MmapLog approach,
which is the fastest for all Multi implementations and also offers persistence
together with a true "logging" approach, where old slots need not to be reused
-- provided there is enough disk space.

Being faster than the other RAM-only solutions might come as a surprise at first,
but the key aspect is that we are not bounded by RAM, thus almost no synchronizations
are needed for buffer reuse.