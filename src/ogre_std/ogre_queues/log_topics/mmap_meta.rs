//! Resting place for the [MMapMeta] `log_topic` 

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_topic::MetaTopic,
};
use std::{
    fs::{OpenOptions,File},
    sync::{
        Arc,
        atomic::{
            AtomicUsize,
            Ordering::Relaxed,
        }
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
    /// the marker where publishers should place new elements
    publisher_tail: AtomicUsize,
    /// the number of elements in `buffer` completely produced and available for consumption
    consumer_tail: AtomicUsize,
    /// the number of allocated elements in `buffer`
    slice_length: AtomicUsize,
    /// the first element of the data in the buffer -- the length is determined by the mmap file size
    first_buffer_element: SlotType,
}


impl<SlotType: Debug> MMapMeta<SlotType> {

    /// Returns the Rust structure able to operate on the mmapped data -- for either reads or writes.\
    /// Even if the mmapped data grow, the returned reference is always valid, as the data never shrinks.
    fn buffer_as_slice_mut(&self) -> &'static mut [SlotType] {
        unsafe {
            let mutable_self = &mut *((self as *const Self) as *mut Self);
            std::slice::from_raw_parts_mut(&mut mutable_self.mmap_contents.first_buffer_element as *mut SlotType, mutable_self.mmap_contents.slice_length.load(Relaxed))
        }
    }

    /// Resizes up the mmap file so more elements are able to fit in it.\
    /// Expansion is done in fixed steps as defined when instantiating this structure.
    fn expand_buffer(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut current_len = self.mmap_file_len.write();
        let new_len = *current_len + self.growing_step_size*std::mem::size_of::<SlotType>() as u64;
        self.mmap_file.set_len(new_len)
            .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not expand buffer of the mentioned mmapped file from {current_len} to {new_len}: {:?}", err, mmap_file_path = self.mmap_file_path))?;
        *current_len = new_len;
        self.mmap_contents.slice_length.fetch_add(self.growing_step_size as usize, Relaxed);
        Ok(())
    }

    pub fn subscribe(self: &Arc<Self>, replay_old_events: bool) -> MMapMetaSubscriber<SlotType> {
        let first_element_slot_id = if replay_old_events {0} else {self.mmap_contents.consumer_tail.load(Relaxed)};
        MMapMetaSubscriber {
            head:                AtomicUsize::new(first_element_slot_id),
            buffer:              self.buffer_as_slice_mut(),
            meta_mmap_log_topic: Arc::clone(&self),
        }
    }

}


impl<SlotType: Debug> MetaTopic<SlotType> for MMapMeta<SlotType> {

    fn new<IntoString: Into<String>>(mmap_file_path: IntoString, growing_step_size: u64) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let mmap_file_path = mmap_file_path.into();
        let mut mmap_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&mmap_file_path)
            .map_err(|err| format!("Could not open file '{mmap_file_path}' (that would be used to mmap a `log_topic` buffer): {:?}", err))?;
        let mmap_file_len = std::mem::size_of::<MMapContents<SlotType>>() as u64 + (growing_step_size-1) * std::mem::size_of::<SlotType>() as u64;  // `MMapContents<SlotType>` already contains 1 slot
        mmap_file.set_len(mmap_file_len)
            .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not set the length of the mentioned mmapped file (after opening it) to {mmap_file_len}: {:?}", err))?;  // related to the size of `MMapContents<SlotType>`
        let mut mmap_handle = unsafe {
            MmapOptions::new()
                .map_mut(&mmap_file)
                .map_err(|err| format!("Couldn't mmap the (already openned) file '{mmap_file_path}' in RW mode, for use as the backing storage of a `log_topic` buffer: {:?}", err))?
        };
        let mmap_contents: &'static mut MMapContents<SlotType> = unsafe { &mut *(mmap_handle.as_mut_ptr() as *mut MMapContents<SlotType>) };
        unsafe { std::ptr::write_bytes::<MMapContents<SlotType>>(mmap_contents, 0, 1); }     // zero-out the header of the mmap file
        mmap_contents.slice_length.store(growing_step_size as usize, Relaxed);
        Ok(Arc::new(Self {
            mmap_file_path,
            mmap_file,
            mmap_file_len: RwLock::new(mmap_file_len),
            mmap_handle,
            mmap_contents,
            growing_step_size,
        }))
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

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let tail = self.mmap_contents.publisher_tail.fetch_add(1, Relaxed);
        let mut buffer = mutable_self.buffer_as_slice_mut();
        if buffer.len() == tail {
            self.expand_buffer().expect("Could not publish to MMAP log");
        }
        setter_fn(&mut buffer[tail]);
        while self.mmap_contents.consumer_tail.compare_exchange_weak(tail, tail+1, Relaxed, Relaxed).is_err() {
            std::hint::spin_loop();
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
    head:                AtomicUsize,
    buffer:              &'static mut [SlotType],
    meta_mmap_log_topic: Arc<MMapMeta<SlotType>>,
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
        let tail = self.meta_mmap_log_topic.mmap_contents.consumer_tail.load(Relaxed);
        // check if there is an element available
        if head >= tail {
            while self.head.compare_exchange_weak(head+1, head, Relaxed, Relaxed).is_err() {
                std::hint::spin_loop();
            }
            return None;
        }
        // check if our `buffer` slice needs syncing with the mmapped data
        if head >= self.buffer.len() {
            mutable_self.buffer = mutable_self.meta_mmap_log_topic.buffer_as_slice_mut();
        }
        let slot_ref = &mut mutable_self.buffer[head];
        Some(getter_fn(slot_ref))

    }

    unsafe fn peek_remaining(&self) -> [&[SlotType]; 2] {
        todo!()
    }
}


/// Unit tests the [mmap_meta](self) module
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;

    /// Simple mmap open/close capabilities
    #[cfg_attr(not(feature = "dox"), test)]
    fn happy_path() {
        #[derive(Debug,PartialEq)]
        struct MyData {
            name: [u8; 254],
            name_len: u8,
            age: u8,
        }
        let mut meta_log_topic = MMapMeta::<MyData>::new("/tmp/mmap_meta.test.mmap", 4096)
            .expect("Instantiating the meta log topic");
        let expected_name = "zertyz";
        let expected_age = 42;

        // enqueue
        //////////

        let publishing_result = meta_log_topic.publish(|slot| {
            for (source, mut target) in expected_name.as_bytes().iter().zip(slot.name.iter_mut()) {
                *target = *source;
            }
            slot.name_len = expected_name.len() as u8;
            slot.age = expected_age;
        }, || false, |_| ());
        assert!(publishing_result, "Publishing failed");

        // dequeue
        //////////

        let consumer_1 = meta_log_topic.subscribe(true);
        // here is one of the essences of a log topic: references exists "forever", as it only grows (old elements are never freed)
        let observed_slot_1: &'static MyData = consumer_1.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                                  || false,
                                                                  |_| {})
            .expect("Consuming the element from `consumer_1`");
        let observed_name = unsafe { std::slice::from_raw_parts(observed_slot_1.name.as_ptr(), observed_slot_1.name_len as usize) };
        let observed_name = String::from_utf8_lossy(observed_name);
        assert_eq!(&observed_name,     expected_name, "Name doesn't match");
        assert_eq!(observed_slot_1.age, expected_age, "Age doesn't match");

        // the second essence of a log topic: new consumers might be created at any time and they will access all previous elements
        let consumer_2 = meta_log_topic.subscribe(true);
        let observed_slot_2: &'static MyData = consumer_2.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                                  || false,
                                                                  |_| {})
            .expect("Consuming the element from `consumer_2`");
        assert_eq!(observed_slot_2 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");

        // consumers may be scheduled not to replay old events
        let consumer_3 = meta_log_topic.subscribe(false);
        let observed_slot_3 = consumer_3.consume(|slot| unsafe { &*(slot as *const MyData) },
                                             || false,
                                             |_| {});
        assert_eq!(None, observed_slot_3, "`consumer_3` was told not to retrieve old elements...");

        // a new enqueue -- available to all consumers
        let publishing_result = meta_log_topic.publish(|slot| {
            for (source, mut target) in expected_name.as_bytes().iter().zip(slot.name.iter_mut()) {
                *target = source.to_ascii_uppercase();
            }
            slot.name_len = expected_name.len() as u8;
            slot.age = 100 - expected_age;
        }, || false, |_| ());
        assert!(publishing_result, "Publishing of a second element failed");
        let observed_slot_1: &'static MyData = consumer_1.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                                  || false,
                                                                  |_| {})
            .expect("Consuming yet another element from `consumer_1`");
        let observed_name = unsafe { std::slice::from_raw_parts(observed_slot_1.name.as_ptr(), observed_slot_1.name_len as usize) };
        let observed_name = String::from_utf8_lossy(observed_name);
        assert_eq!(*observed_name, expected_name.to_ascii_uppercase(), "Name doesn't match");
        assert_eq!(100 - observed_slot_1.age, expected_age, "Age doesn't match");
        let observed_slot_2: &'static MyData = consumer_2.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                                  || false,
                                                                  |_| {})
            .expect("Consuming yet another element from `consumer_2`");
        assert_eq!(observed_slot_2 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");
        let observed_slot_3: &'static MyData = consumer_3.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                                  || false,
                                                                  |_| {})
            .expect("Consuming the element from `consumer_3`");
        assert_eq!(observed_slot_3 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");

    }
}