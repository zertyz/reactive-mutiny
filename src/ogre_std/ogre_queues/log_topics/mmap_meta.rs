//! Resting place for the [MMapMeta] `log_topic` 

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_topic::MetaTopic,
};
use std::{
    fs::{OpenOptions,File},
    sync::atomic::{
        AtomicUsize,
        Ordering::Relaxed,
    },
    fmt::Debug,
};
use memmap::{
    MmapOptions,
    MmapMut,
};
use parking_lot::RwLock;

/// Basis for multiple producer / multiple consumer `log_topics`, using an m-mapped file as the backing storage --
/// which always grows and have the ability to create consumers able to replay all elements ever created whenever a new subscriber is instantiated.
/// Synchronization is done throug simple atomics, making this container the fastest among [full_sync_queues::full_sync_meta] & [atomic_sync_queues::atomic_sync_meta].
pub struct MMapMeta<SlotType: 'static> {

    mmap_file_path: String,
    mmap_file: File,
    mmap_file_len: RwLock<u64>,
    mmap_handle: MmapMut,
    mmap_contents: &'static mut MMapContents<SlotType>,
    growing_step_size: u64,
}

/// the data represented on the mmap-file
#[repr(C)]
struct MMapContents<SlotType> {
    /// the length of elements in `buffer` completely produced and available for consumption
    tail: AtomicUsize,
    /// the number of allocated elements in `buffer`
    slice_length: AtomicUsize,
    /// the first element of the data in the buffer -- the length is determined by the mmap file size
    first_buffer_element: SlotType,
}


impl<SlotType: Debug> MMapMeta<SlotType> {

    /// Returns the Rust structure able to operate on the mmapped data -- for either reads or writes.\
    /// Even if the mmapped data grow, the returned reference is always valid, as the data never shrinks.
    fn buffer_as_slice_mut(&mut self) -> &'static mut [SlotType] {
        unsafe { std::slice::from_raw_parts_mut(&mut self.mmap_contents.first_buffer_element as *mut SlotType, self.mmap_contents.slice_length.load(Relaxed)) }
    }

    /// Resizes up the mmap file so more elements are able to fit in it.\
    /// Expansion is done in fixed steps as defined when instantiating this structure.
    fn expand_buffer(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut current_len = self.mmap_file_len.write();
        let new_len = *current_len + self.growing_step_size*std::mem::size_of::<SlotType>() as u64;
        self.mmap_file.set_len(new_len)
            .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not expand buffer of the mentioned mmapped file from {current_len} to {new_len}: {:?}", err, mmap_file_path = self.mmap_file_path))?;
        *current_len = new_len;
        Ok(())
    }

}


impl<SlotType: Debug> MetaTopic<SlotType> for MMapMeta<SlotType> {

    fn new<IntoString: Into<String>>(mmap_file_path: IntoString, growing_step_size: u64) -> Result<Self, Box<dyn std::error::Error>> {
        let mmap_file_path = mmap_file_path.into();
        let mut mmap_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&mmap_file_path)
            .map_err(|err| format!("Could not open file '{mmap_file_path}' (that would be used to mmap a `log_topic` buffer): {:?}", err))?;
        let mmap_file_len = 2 * std::mem::size_of::<u64>() as u64 + growing_step_size * std::mem::size_of::<SlotType>() as u64;  // related to the size of `MMapContents<SlotType>`
        mmap_file.set_len(mmap_file_len)
            .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not set the length of the mentioned mmapped file (after opening it) to {mmap_file_len}: {:?}", err))?;  // related to the size of `MMapContents<SlotType>`
        let mut mmap_handle = unsafe {
            MmapOptions::new()
                .map_mut(&mmap_file)
                .map_err(|err| format!("Couldn't mmap the (already openned) file '{mmap_file_path}' in RW mode, for use as the backing storage of a `log_topic` buffer: {:?}", err))?
        };
        let mmap_contents: &'static mut MMapContents<SlotType> = unsafe { &mut *(mmap_handle.as_mut_ptr() as *mut MMapContents<SlotType>) };
        Ok(Self {
            mmap_file_path,
            mmap_file,
            mmap_file_len: RwLock::new(mmap_file_len),
            mmap_handle,
            mmap_contents,
            growing_step_size,
        })
    }

    // fn subscribe(&self) -> MMapMetaSubscriber<SlotType> {
    //
    // }

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

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let tail = self.mmap_contents.tail.load(Relaxed);
        let mut buffer = mutable_self.buffer_as_slice_mut();
        if buffer.len() == tail {
            self.expand_buffer().expect("Could not publish to MMAP log");
        }
        true
    }

    fn available_elements(&self) -> usize {
        todo!()
    }

    fn max_size(&self) -> usize {
        todo!()
    }

    fn debug_info(&self) -> String {
        todo!()
    }
}

pub struct MMapMetaSubscriber<SlotType: 'static> {
    head:              AtomicUsize,
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

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let head = self.head.fetch_add(1, Relaxed);
        let slot_ref = &mut mutable_self.buffer[head];
        Some(getter_fn(slot_ref))

    }

    unsafe fn peek_remaining(&self) -> [&[SlotType]; 2] {
        todo!()
    }
}