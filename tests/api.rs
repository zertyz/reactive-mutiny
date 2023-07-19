//! Tests some rather "advanced" API usages for the [reactive_mutiny] library that
//! that are too complicated or simply don't have enough didactics to be part of the "examples" set.
//!
//! NOTE: many "tests" here have no real assertions, as they are used only to verify the API is able to represent certain models


use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use futures::StreamExt;
use reactive_mutiny::prelude::advanced::{
    UniZeroCopyAtomic,
    Instruments,
};


#[ctor::ctor]
fn suite_setup() {
    simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
}

/// Checks that our channels (for [Uni]s or [Multi]s) may be used in elaborated generics expansions.\
/// Observe here that the `pipeline_builder` function, in order to keep a simple signature, shouldn't be passed down the calling chain:
/// instead, a [Uni]/[Multi] must be created on the first level, or else the code won't compile (as Rust wouldn't be able to resolve the intricate generic typings resulting from that)
#[cfg_attr(not(doc), tokio::test)]
async fn elaborated_generics_on_call_chains() {

    // generics typing
    const INSTRUMENTS: usize = Instruments::LogsWithExpensiveMetrics.into();
    const BUFFER_SIZE: usize = 1024;
    type Uni<ItemType> = UniZeroCopyAtomic<ItemType, BUFFER_SIZE, 1, INSTRUMENTS>;
    // type Uni<ItemType> = UniMoveAtomic<ItemType, BUFFER_SIZE, 1, INSTRUMENTS>;

    trait CustomTrait<T> { fn get_data(self) -> T; }
    #[derive(Debug)]
    struct CustomType<T: Debug> {data: T}
    impl<T: Debug> CustomTrait<CustomType<T>> for CustomType<T> { fn get_data(self) -> CustomType<T> { CustomType {data: self.data} } }
    async fn with_generics<RemoteMessages: CustomTrait<RemoteMessages> + Send + Sync + Debug>
                          (events_sender: Arc<Uni<RemoteMessages>>,
                           value:         RemoteMessages)
                          -> bool {
        assert!(events_sender.try_send(|slot| *slot = value).is_none(), "couldn't send value");
        events_sender.close(Duration::from_millis(100)).await
    }

    // notice the simple `pipeline_builder`, that is able to make it down the chain of `with_generics()` call only in the `Uni` / `Multi` form
    // (as opposed to passing the `pipeline_builder` as a parameter to `with_generics()`, which would, then, create the `Uni` / `Multi` there)
    let uni = Uni::<CustomType<bool>>::new("advanced generics")
            .spawn_non_futures_non_fallibles_executors(1,
                                                       |remote_messages_stream| remote_messages_stream.map(|_in_msg| CustomType {data: true}),
                                                       |_executor| async {});
    with_generics(uni, CustomType {data: false}).await;

}
