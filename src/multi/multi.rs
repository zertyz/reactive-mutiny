//! See [super]

use super::super::instruments::Instruments;
use std::{
    sync::Arc,
    fmt::Debug,
    pin::Pin,
    time::Duration,
};
use indexmap::IndexMap;
use futures::{Stream};
use tokio::{
    sync::{RwLock},
};


/// this is the fastest [MultiChannel] for general use, as revealed in performance tests
type MultiChannelType<ItemType,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> = channels::ogre_mpmc_queue::OgreMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS>;


/// Contains the producer-side [Multi] handle used to interact with the multi event
/// -- for closing the stream, requiring stats, ...
pub struct Multi<ItemType:          Clone + Unpin + Send + Sync + Debug,
                 const BUFFER_SIZE: usize,
                 const MAX_STREAMS: usize,
                 const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> {
    pub stream_name:    String,
    pub channel:        Arc<Pin<Box<MultiChannelType<ItemType, BUFFER_SIZE, MAX_STREAMS>>>>,
    pub executor_infos: RwLock<IndexMap<String, ExecutorInfo<INSTRUMENTS>>>,
}

impl<ItemType:          Clone + Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize,
     const INSTRUMENTS: usize>
Multi<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {

    pub fn new<IntoString: Into<String>>(stream_name: IntoString) -> Self {
        let stream_name = stream_name.into();
        Multi {
            stream_name:    stream_name.clone(),
            channel:        MultiChannelType::<ItemType, BUFFER_SIZE, MAX_STREAMS>::new(stream_name.clone()),
            executor_infos: RwLock::new(IndexMap::new()),
        }
    }

    pub fn stream_name<'r>(self: &'r Self) -> &'r String {
        &self.stream_name
    }

    pub fn listener_stream(&self) -> (impl Stream<Item=Arc<ItemType>>, u32) {
        self.channel.listener_stream()
    }

    pub async fn add_executor(&self, stream_executor: Arc<StreamExecutor<INSTRUMENTS>>, stream_id: u32) -> Result<(), Box<dyn std::error::Error>> {
        let mut internal_multis = self.executor_infos.write().await;
        if internal_multis.contains_key(&stream_executor.executor_name()) {
            Err(Box::from(format!("Can't add a new pipeline to a Multi: an executor with the same name is already present: '{}'", stream_executor.executor_name())))
        } else {
            internal_multis.insert(stream_executor.executor_name(), ExecutorInfo { stream_executor, stream_id });
            Ok(())
        }
    }

    /// Asynchronously blocks until all resources associated with the executor responsible for `pipeline_name` are freed:
    ///   1) immediately causes `pipeline_name` to cease receiving new elements by removing it from the active list
    ///   2) wait for all pending elements to be processed
    ///   3) remove the queue/channel and wake the Stream to see that it has ended
    ///   4) waits for the executor to inform it ceased its execution
    ///   5) return, dropping all resources\
    /// Note it might make sense to spawn this operation by a `Tokio task`, for it may block indefinitely if the Stream has no timeout.\
    /// Also note that timing out this operation is not advisable, for resources won't be freed until it reaches the last step.\
    /// Returns false if there was no executor associated with `pipeline_name`.
    pub async fn flush_and_cancel_executor<IntoString: Into<String>>
                                          (self:          &Self,
                                           pipeline_name: IntoString,
                                           timeout:       Duration) -> bool {

        let executor_name = format!("{}: {}", self.stream_name, pipeline_name.into());
        // remove the pipeline from the active list
        let mut executor_infos = self.executor_infos.write().await;
        let executor_info = match executor_infos.remove(&executor_name) {
            Some(executor) => executor,
            None => return false,
        };
        drop(executor_infos);

        // wait until all elements are taken out from the queue
        executor_info.stream_executor.report_scheduled_to_finish();
        self.channel.end_stream(executor_info.stream_id, timeout).await;
        true
    }

    #[inline(always)]
    pub fn send(&self, item: ItemType) {
        self.channel.send(item);
    }

    #[inline(always)]
    pub fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.channel.pending_items_count()
    }

    // pub async fn report_executor_ended(&self, _executor: &Arc<StreamExecutor<LOG, METRICS>>) {
    //     self.multi_state.guarded_state.lock().await.consumption_done = true;
    //     // TODO: it makes more sense to count the closed executors
    //     // TODO: we surely need to merge counters & averages together -- maybe average merging should be done in the rolling average module
    //     //       ... or maybe we should keep them separate and join when stats is requested -- which would work before the streams are closed too
    //     //       (the above line depends on the TO-DO from this trait mentioning this trait is the place for the stats gathering method)
    // }

    /// closes this multi, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// If this multi share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [multis_close_async!()]
    pub async fn close(self: &Self, timeout: Duration) {
        self.channel.end_all_streams(timeout).await;
    }

}

pub struct ExecutorInfo<const INSTRUMENTS: usize> {
    pub stream_executor: Arc<StreamExecutor<INSTRUMENTS>>,
    pub stream_id: u32,
}

/// Macro to close, atomically, all [Multi]s passed as parameters
/// TODO the closer may receive a time limit to wait -- returning how many elements were still there after if gave up waiting
/// TODO busy wait ahead -- is it possible to get rid of that without having to poll & sleep?
///      (anyway, this function is not meant to be used in production -- unless when quitting the app... so this is not a priority)
#[macro_export]
macro_rules! multis_close_async {
    ($timeout: expr,
     $($multi: expr),+) => {
        {
            tokio::join!( $( $multi.channel.flush($timeout), )+ );
            tokio::join!( $( $multi.channel.end_all_streams($timeout), )+ );
        }
    }
}
pub use multis_close_async;
use crate::{
    multi::channels,
    stream_executor::StreamExecutor,
};
