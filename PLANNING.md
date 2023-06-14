# Foreseable evolutions to this project

This file contains the backlog & implementation plans for all foreseable features to be added / fixes to this project

## Prefixes

  - (b) bug fix for broken functionalities
  - (f) new functional requisite
  - (n) new non-functional requisite
  - (r) internal re-engineering / refactor / improvement, to increase speed and/or enable further progress to be done cheaper

# TODO

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
  r12) 2023-06-14: Unify `UniBuilder` & `Uni`, for types simplification & symmetry with how `Multi`s work -- causing a MAJOR version upgrade

# Done

  0) 2022/2023: ((past improvements were not documented here yet))
  r1) 2023-05-30: Images used in the README must be .png rather than .svg, so they may appear in crates.io
  f10) 2023-06-05: Unis must be able to process all the combinations of fallible/non-fallible & futures/non-futures -- the same for Multi
  b11) 2023-06-14: there seems to be no known way of passing the stream processing logic down a complex call chain
