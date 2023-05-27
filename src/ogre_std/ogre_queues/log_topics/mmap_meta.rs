//! Resting place for the [MMapMeta] `log_topic` 

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_topic::MetaTopic,
};
use std::{fs::{OpenOptions, File}, sync::{
    Arc,
    atomic::{
        AtomicUsize,
        Ordering::Relaxed,
    }
}, fmt::Debug, ptr};
use std::num::NonZeroU32;
use memmap::{
    MmapOptions,
    MmapMut,
};
use parking_lot::RwLock;

/// Basis for multiple producer / multiple consumer `log_topics`, using an m-mapped file as the backing storage --
/// which always grows and have the ability to create consumers able to replay all elements ever created whenever a new subscriber is instantiated.
/// Synchronization is done through simple atomics, making this container the fastest among [full_sync_queues::full_sync_meta] & [atomic_sync_queues::atomic_sync_meta].\
///
/// NOTE: applications using this container should be designed to be restarted from time-to-time (daily restarts are recommended) -- or to implement clever ways of synchronizing the dropping of old & recreation of new instances
///       (to do this on-the-fly), as dropping used instances is the only way to free the disk resources used by the m-mappings present here.
///       If your app is a long-runner that don't need the "replay all events guarantee", please use [ring_buffer_topics] instead.
///
/// NOTE 2: this container was not designed to survive app crashes nor to provide inter-process communications: it, currently, only allows the application to store a greater-than-RAM number of events in a zero-cost manner,
///         with none to minimal performance hit. Upgrading it seems to be a no-brainer, though, if these ever prove to be useful features.
#[derive(Debug)]
pub struct MMapMeta<'a, SlotType: 'a> {

    mmap_file_path: String,
    mmap_file: File,
    mmap_handle: MmapMut,
    mmap_contents: &'a mut MMapContents<SlotType>,
    buffer:        &'a mut [SlotType],
}

/// the data represented on the mmap-file
#[repr(C)]
#[derive(Debug)]
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


impl<'a, SlotType: 'a + Debug> MMapMeta<'a, SlotType> {

    /// Returns the Rust structure able to operate on the mmapped data -- for either reads or writes.\
    /// Even if the mmapped data grow, the returned reference is always valid, as the data never shrinks.
    fn buffer_as_slice_mut(&self) -> &'a mut [SlotType] {
        unsafe {
            let mutable_self = &mut *((self as *const Self) as *mut Self);
            std::slice::from_raw_parts_mut(&mut mutable_self.mmap_contents.first_buffer_element as *mut SlotType, mutable_self.mmap_contents.slice_length.load(Relaxed))
        }
    }

    /// Returns a single subscriber -- for past events only. Even if new events arrive, the subscriber won't see them
    /// -- it is safe to assume that all old events were supplied when the first `None` is returned by the subscriber
    pub fn subscribe_to_old_events_only(self: &Arc<Self>) -> MMapMetaSubscriber<'a, SlotType> {
        todo!()
    }

    /// Returns a single subscriber -- for forthcoming events only
    pub fn subscribe_to_new_events_only(self: &Arc<Self>) -> MMapMetaSubscriber<'a, SlotType> {
        let first_element_slot_id = self.mmap_contents.consumer_tail.load(Relaxed);
        MMapMetaSubscriber {
            head:                AtomicUsize::new(first_element_slot_id),
            buffer:              self.buffer_as_slice_mut(),
            meta_mmap_log_topic: Arc::clone(&self),
        }
    }

    /// Returns two subscribers:
    ///   - one for the past events (that, once exhausted, won't see any of the forthcoming events)
    ///   - another for the forthcoming events.
    /// The split is guaranteed not to miss any events: no events will be lost between the last of the "past" and
    /// the first of the "forthcoming" events
    pub fn subscribe_to_separated_old_and_new_events(self: &Arc<Self>) -> (MMapMetaSubscriber<'a, SlotType>, MMapMetaSubscriber<'a, SlotType>) {
        todo!()
    }

    /// Returns a single subscriber containing both old and new events -- notice that, with this method,
    /// there is no way of discriminating where the "old" events end and where the "new" events start.
    pub fn subscribe_to_joined_old_and_new_events(self: &Arc<Self>) -> MMapMetaSubscriber<'a, SlotType> {
        let first_element_slot_id = 0;
        MMapMetaSubscriber {
            head:                AtomicUsize::new(first_element_slot_id),
            buffer:              self.buffer_as_slice_mut(),
            meta_mmap_log_topic: Arc::clone(&self),
        }
    }

}


impl<'a, SlotType: 'a + Debug> MetaTopic<'a, SlotType> for MMapMeta<'a, SlotType> {

    /// Instantiates a new **mmap based** *meta log_topic* using the given file as the backing storage -- which may grow bigger than RAM and will have all its contents erased before starting.
    ///   - `mmap_file_path` will have all its space sparsely pre-allocated -- so it should be used on ext4, btrfs, etc.
    ///   - `max_slots` should be way-over-the-maximum-number-of-elements expected to be produced throughout the life of this object (as log topics allows the full replay of events, it only grows and slots are never reused).
    /// A reasonable number would be 1T (1<<40) number of elements, which (if ever used) is likely to exceed the storage of most VPSes.\
    ///
    /// Performance & operational considerations:
    ///   1. If BTRFS is in use, set the mmapped files to not be compressed and to have Copy-On-Write disabled: `truncate -s 0 /DIR/*.mmap; chattr +Cm /DIR/*.mmap`. Check with `lsattr /DIR/*.mmap`
    ///   2. Having a high swappiness may cause unwanted swaps in high usage scenarios. Consider decreasing it to `echo 1 | sudo tee /proc/sys/vm/swappiness`
    ///   3. The mmapped files will have a huge size, as reported by `ls -l`. To see the real usage, `du /DIR/*.mmap` and `compsize /DIR/*.mmap`
    ///   4. When measuring performance on a BTRFS filesystem, remember it has some services on the background. To minimize the block freeing service impact, do this between runs: `truncate -s 0 /DIR/*.mmap; sudo sync`
    fn new<IntoString: Into<String>>(mmap_file_path: IntoString, max_slots: u64) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let mmap_file_path = mmap_file_path.into();
        let mmap_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&mmap_file_path)
            .map_err(|err| format!("Could not open file '{mmap_file_path}' (that would be used to mmap a `log_topic` buffer): {:?}", err))?;
        let mmap_file_len = std::mem::size_of::<MMapContents<SlotType>>() as u64 + (max_slots-1) * std::mem::size_of::<SlotType>() as u64;  // `MMapContents<SlotType>` already contains 1 slot
        mmap_file.set_len(0)
            .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not set the (sparsed) length of the mentioned mmapped file (after opening it) to {mmap_file_len}: {:?}", err, mmap_file_len = 0))?;
        mmap_file.set_len(mmap_file_len as u64)
             .map_err(|err| format!("mmapped log topic '{mmap_file_path}': Could not set the (sparsed) length of the mentioned mmapped file (after opening it) to {mmap_file_len}: {:?}", err))?;
        let mut mmap_handle = unsafe {
            MmapOptions::new()
                .len(mmap_file_len as usize)
                .map_mut(&mmap_file)
                .map_err(|err| format!("Couldn't mmap the (already openned) file '{mmap_file_path}' in RW mode, for use as the backing storage of a `log_topic` buffer: {:?}", err))?
        };
        let mmap_contents: &mut MMapContents<SlotType> = unsafe { &mut *(mmap_handle.as_mut_ptr() as *mut MMapContents<SlotType>) };
        unsafe { std::ptr::write_bytes::<MMapContents<SlotType>>(mmap_contents, 0, 1); }     // zero-out the header of the mmap file... test to confirm the ahead zeroing isn't really necessary
        mmap_contents.slice_length.store(max_slots as usize, Relaxed);
        mmap_contents.publisher_tail.store(0, Relaxed);
        mmap_contents.consumer_tail.store(0, Relaxed);
        let buffer_slice_ptr = &mut mmap_contents.first_buffer_element as *mut SlotType;
        let slice_length = mmap_contents.slice_length.load(Relaxed);
        Ok(Arc::new(Self {
            mmap_file_path,
            mmap_file,
            mmap_handle,
            mmap_contents,
            buffer: unsafe { std::slice::from_raw_parts_mut(buffer_slice_ptr, slice_length) },
        }))
    }

}

impl<'a, SlotType: 'a + Debug> MetaPublisher<'a, SlotType> for MMapMeta<'a, SlotType> {

    #[inline(always)]
    fn publish<F: FnOnce(&mut SlotType)>(&self, setter: F) -> Option<NonZeroU32> {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let tail = self.mmap_contents.publisher_tail.fetch_add(1, Relaxed);
        let slot = unsafe { mutable_self.buffer.get_unchecked_mut(tail) };
        setter(slot);
        while self.mmap_contents.consumer_tail.compare_exchange_weak(tail, tail+1, Relaxed, Relaxed).is_err() {
            std::hint::spin_loop();
        }
        NonZeroU32::new(1 + tail as u32)
    }

    #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> Option<NonZeroU32> {
        self.publish(|slot| *slot = item)
    }

    fn leak_slot(&self) -> Option<(/*ref:*/ &'a mut SlotType, /*id: */u32)> {
        todo!()
    }

    fn publish_leaked_ref(&'a self, slot: &'a SlotType) -> Option<NonZeroU32> {
        todo!()
    }

    fn publish_leaked_id(&'a self, slot_id: u32) -> Option<NonZeroU32> {
        todo!()
    }

    fn unleak_slot_ref(&'a self, slot: &'a mut SlotType) {
        todo!()
    }

    fn unleak_slot_id(&'a self, slot_id: u32) {
        todo!()
    }

    #[inline(always)]
    fn available_elements_count(&self) -> usize {
        self.mmap_contents.publisher_tail.load(Relaxed)
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        self.mmap_contents.slice_length.load(Relaxed)
    }

    fn debug_info(&self) -> String {
        todo!()
    }
}

pub struct MMapMetaSubscriber<'a, SlotType: 'a> {
    head:                AtomicUsize,
    buffer:              &'a mut [SlotType],
    meta_mmap_log_topic: Arc<MMapMeta<'a, SlotType>>,
}

impl<'a, SlotType: 'a + Debug> MetaSubscriber<'a, SlotType> for MMapMetaSubscriber<'a, SlotType> {

    fn consume<GetterReturnType: 'a,
               GetterFn:                   FnOnce(&'a SlotType) -> GetterReturnType,
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
            report_empty_fn();
            return None;
        }
        let slot_ref = unsafe { mutable_self.buffer.get_unchecked(head) };
        report_len_after_dequeueing_fn((tail - head) as i32);
        Some(getter_fn(slot_ref))
    }

    fn consume_leaking(&'a self) -> Option<(/*ref:*/ &'a SlotType, /*id: */u32)> {
        todo!()
    }

    fn release_leaked_ref(&'a self, slot: &'a SlotType) {
        todo!()
    }

    fn release_leaked_id(&'a self, slot_id: u32) {
        todo!()
    }

    #[inline(always)]
    fn remaining_elements_count(&self) -> usize {
        self.meta_mmap_log_topic.mmap_contents.consumer_tail.load(Relaxed) - self.head.load(Relaxed)
    }

    unsafe fn peek_remaining(&self) -> Vec<&SlotType> {
        todo!()
    }
}


/// Unit tests the [mmap_meta](self) module
#[cfg(any(test,doc))]
mod tests {
    use super::*;

    /// Simple mmap open/close capabilities
    #[cfg_attr(not(doc),test)]
    fn happy_path() {
        #[derive(Debug,PartialEq)]
        struct MyData {
            name: [u8; 254],
            name_len: u8,
            age: u8,
        }
        let meta_log_topic = MMapMeta::<MyData>::new("/tmp/happy_path.test.mmap", 4096)
            .expect("Instantiating the meta log topic");
        let expected_name = "zertyz";
        let expected_age = 42;

        // enqueue
        //////////

        let mut iter = expected_name.as_bytes().iter();
        let my_data = MyData {
            name: [0; 254].map(|_| *iter.next().unwrap_or(&0)),
            name_len: expected_name.len() as u8,
            age: expected_age,
        };
        let publishing_result = meta_log_topic.publish(|slot| *slot = my_data);
        assert!(publishing_result.is_some(), "Publishing failed");

        // dequeue
        //////////

        let consumer_1 = meta_log_topic.subscribe_to_joined_old_and_new_events();
        // here is one of the essences of a log topic: references exists "forever", as it only grows (old elements are never freed)
        let observed_slot_1: &MyData = consumer_1.consume(|slot| unsafe { &*(slot as *const MyData) },
                                                          || false,
                                                          |_| {})
            .expect("Consuming the element from `consumer_1`");
        let observed_name = unsafe { std::slice::from_raw_parts(observed_slot_1.name.as_ptr(), observed_slot_1.name_len as usize) };
        let observed_name = String::from_utf8_lossy(observed_name);
        assert_eq!(&observed_name,     expected_name, "Name doesn't match");
        assert_eq!(observed_slot_1.age, expected_age, "Age doesn't match");

        // the second essence of a log topic: new consumers might be created at any time and they will access all previous elements
        let consumer_2 = meta_log_topic.subscribe_to_joined_old_and_new_events();
        let observed_slot_2: &MyData = consumer_2.consume(|slot| unsafe {&*(slot as *const MyData)},
                                                          || false,
                                                          |_| {})
            .expect("Consuming the element from `consumer_2`");
        assert_eq!(observed_slot_2 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");

        // consumers may be scheduled not to replay old events
        let consumer_3 = meta_log_topic.subscribe_to_new_events_only();
        let observed_slot_3 = consumer_3.consume(|slot| unsafe {&*(slot as *const MyData)},
                                             || false,
                                             |_| {});
        assert_eq!(None, observed_slot_3, "`consumer_3` was told not to retrieve old elements...");

        // a new enqueue -- available to all consumers
        let mut iter = expected_name.as_bytes().iter().map(|c| c.to_ascii_uppercase());
        let my_data = MyData {
            name: [0; 254].map(|_| iter.next().unwrap_or(0)),
            name_len: expected_name.len() as u8,
            age: 100 - expected_age,
        };
        let publishing_result = meta_log_topic.publish(|slot| *slot = my_data);
        assert!(publishing_result.is_some(), "Publishing of a second element failed");
        let observed_slot_1: &MyData = consumer_1.consume(|slot| unsafe {&*(slot as *const MyData)},
                                                          || false,
                                                          |_| {})
            .expect("Consuming yet another element from `consumer_1`");
        let observed_name = unsafe { std::slice::from_raw_parts(observed_slot_1.name.as_ptr(), observed_slot_1.name_len as usize) };
        let observed_name = String::from_utf8_lossy(observed_name);
        assert_eq!(*observed_name, expected_name.to_ascii_uppercase(), "Name doesn't match");
        assert_eq!(100 - observed_slot_1.age, expected_age, "Age doesn't match");
        let observed_slot_2: &MyData = consumer_2.consume(|slot| unsafe {&*(slot as *const MyData)},
                                                          || false,
                                                          |_| {})
            .expect("Consuming yet another element from `consumer_2`");
        assert_eq!(observed_slot_2 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");
        let observed_slot_3: &MyData = consumer_3.consume(|slot| unsafe {&*(slot as *const MyData)},
                                                          || false,
                                                          |_| {})
            .expect("Consuming the element from `consumer_3`");
        assert_eq!(observed_slot_3 as *const MyData, observed_slot_1 as *const MyData, "These references should point to the same address");

    }

    /// Checks the mmap file would be able to fill all the storage, if needed
    #[cfg_attr(not(doc),test)]
    fn indefinite_growth() {

        const N_ELEMENTS: u64 = 2 * 1024 * 1024;

        #[derive(Debug, PartialEq)]
        struct MyData {
            id:                          u64,
            year_of_birth:               u16,
            year_of_death:               u16,
            known_number_of_descendents: u32,
        }

        let meta_log_topic = MMapMeta::<MyData>::new("/tmp/indefinite_growth.test.mmap", 1024 * 1024 * 1024)
            .expect("Instantiating the meta log topic");
        let consumer = meta_log_topic.subscribe_to_new_events_only();

        // enqueue
        //////////
        for i in 0..N_ELEMENTS {
            let publishing_result = meta_log_topic.publish(|slot| *slot = create_expected_element(i));
            assert!(publishing_result.is_some(), "Publishing of event #{i} failed");
        }

        // dequene & assert
        ///////////////////
        for i in 0..N_ELEMENTS {
            let expected_element = create_expected_element(i);
            let observed_element = consumer.consume(|slot| unsafe {&*(slot as *const MyData)},
                               || false,
                               |_| {})
                .expect("Consuming an element");
            assert_eq!(observed_element, &expected_element, "Element #{i} doesn't match");
        }

        fn create_expected_element(i: u64) -> MyData {
            let year_of_birth = 1 + i as u16 % 2023;
            let year_of_death = year_of_birth + (i % 120) as u16;
            MyData {
                id: i,
                year_of_birth,
                year_of_death,
                known_number_of_descendents: ( (i % year_of_birth as u64) * (year_of_death - year_of_birth) as u64 ) as u32,
            }
        }

    }

    /// Checks that dropping leaves no dangling references behind
    #[cfg_attr(not(doc),test)]
    fn safe_lifetimes<'a>() {
        const EXPECTED_ELEMENT: u128 = 1928384756;

        let meta_log_topic = MMapMeta::<u128>::new("/tmp/safe_lifetimes.test.mmap", 1024 * 1024 * 1024)
            .expect("Instantiating the meta log topic");
        let consumer = meta_log_topic.subscribe_to_new_events_only();
        meta_log_topic.publish(|slot| *slot = EXPECTED_ELEMENT);
        let observed_element = consumer.consume(|slot| unsafe {&*(slot as *const u128)},
                                                || false,
                                                |_| {})
            .expect("Consuming an element");
        assert_eq!(observed_element, &EXPECTED_ELEMENT, "Dequeued element doesn't match");

        // here you may either comment the drop or the assert, but not both -- meaning the unsafe code is correctly respecting lifetimes
        drop(consumer);
        //assert_eq!(observed_element, &EXPECTED_ELEMENT, "Dequeued element doesn't match");
    }

}