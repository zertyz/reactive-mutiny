//! Resting place for the [MMapMeta] `log_topic` 

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
};
use std::{
    fs::{OpenOptions,File},
    sync::atomic::{
        AtomicU64,
        Ordering::Relaxed,
    },
};
use memmap::{
    MmapOptions,
    MmapMut,
};

/// Basis for multiple producer / multiple consumer `log_topics`, using an m-mapped file as the backing storage --
/// which always grows and have the ability to create consumers able to replay all elements ever created whenever a new subscriber is instantiated.
/// Synchronization is done throug simple atomics, making this container the fastest among [full_sync_queues::full_sync_meta] & [atomic_sync_queues::atomic_sync_meta].
pub struct MMapMeta<SlotType> {

    mmap_file_path: String,
    mmap_file: File,
    mmap_handle: MmapMut,
    mmap_contents: &'static MMapContents<SlotType>,
    growing_step_size: usize,
}

/// the data represented on the mmap-file
#[repr(C)]
struct MMapContents<SlotType> {
    /// the length of elements in `buffer` completely produced and available for consumption
    tail: AtomicU64,
    /// the number of allocated elements in `buffer`
    slice_length: AtomicU64,
    /// the data
    buffer: [SlotType],
}


impl<SlotType: Debug> MMapMeta<SlotType> {

    /// Returns the Rust structure able to operate on the mmapped data -- for either reads or writes.\
    /// Even if the mmapped data grow, the returned reference is always valid, as the data never shrinks.
    fn buffer_as_slice_mut(&self) -> &'static mut [SlotType] {
        unsafe { std::slice::from_raw_parts_mut(self.mmap_contents.buffer as *mut [SlotType], self.mmap_contents.slice_length.load(Relaxed)) }
    }

    /// Resizes up the mmap file so more elements are able to fit in it.\
    /// Expansion is done in fixed steps as defined when instantiating this structure.
    fn expand_buffer(&self) {
        self.mmap_file.set_length(self.mmap_file.get_length() + self.growing_step_size*std::size_of::<SlotType>());
    }

}


impl<SlotType: Debug> MetaTopic<SlotType> for MMapMeta<SlotType> {

    fn new<IntoString: Into<String>>(mmap_file_path: IntoString, growing_step_size: usize) -> Result<Self, Box<&dyn std::error::Error>> {
        let mmap_file_path = mmap_file_path.into();
        let mut mmap_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&mmap_file_path)
            .map_err(|err| Box::from(format!("Could not open file '{mmap_file_path}' (that would be used to mmap a `log_topic` buffer): {:?}", err)))?;
        mmap_file.set_len(2 * std::size_of(u64) + growing_step_size * std::size_of::<SlotType>());  // related to the size of `MMapContents<SlotType>`
        let mmap_handle = MmapOptions::new()
            .map_mut(&mmap_file)
            .map_err(|err| Box::from(format!("Couldn't mmap the (already openned) file '{mmap_file_path}' in RW mode, for use as the backing storage of a `log_topic` buffer: {:?}", err)))?;
        let mmap_contents: &MMapContents<SlotType> = mmap_handle.as_mut_ptr() as &MMapContents<SlotType>;
        MMapMeta {
            mmap_file_path,
            mmap_file,
            mmap_handle,
            mmap_contents,
            growing_step_size,
        }
    }

    fn subscribe(&self) -> MMapMetaSubscriber<SlotType> {

    }

}

impl<SlotType: Debug> MetaPublisher<SlotType> for MMapMeta<SlotType> {

    fn publish<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool {

        let tail = self.mmap_contents.tail.load(Relaxed);
        let mut buffer = self.buffer_as_slice_mut();
        if buffer.len() == tail {
            self.expand_buffer();
        }
    }

}

pub struct MMapMetaSubscriber<SlotType> {
    head:              AtomicU64,
    buffer:            &'static mut [SlotType],
}

impl<SlotType: Debug> MetaSubscriber<SlotType> for MMapMetaSubscriber<SlotType> {

    fn consume<GetterReturnType,
               GetterFn:                   Fn(&mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        let head = self.head.fetch_add(1, Relaxed);
        let slot_ref = self.buffer[head];
        getter_fn(slot_ref)

    }

}