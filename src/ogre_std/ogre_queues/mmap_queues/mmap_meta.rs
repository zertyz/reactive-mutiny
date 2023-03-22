//! Basis for multiple producer / multiple consumer queues using an m-mapped file as the backing storage, allowing the creation of isolated consumers.
//! (making it a "log queue", which always grows and have the ability to create consumers able to replay all elements since it was created).
//! Synchronization is done like in [full_sync_queues::full_sync_meta]
//! full synchronization through an atomic flag, with a clever & experimentally tuned efficient locking mechanism.

