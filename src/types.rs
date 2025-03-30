use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use solana_sdk::packet;

pub type ShredInfo = (usize, [u8; packet::PACKET_DATA_SIZE]);
pub const SHRED_SIZE: usize = packet::PACKET_DATA_SIZE;

pub const CAPACITY: usize = 1 << 12;
pub struct RingSlotStore<T> {
    store: Box<[T; CAPACITY]>,
    max_slot: AtomicU64,
}

impl<T: Default + Debug> RingSlotStore<T> {
    pub fn new() -> Self {
        let mut vec = Vec::with_capacity(CAPACITY);
        for _ in 0..CAPACITY {
            vec.push(T::default());
        }

        let store = vec.into_boxed_slice();
        let store = unsafe {
            Box::from_raw(Box::into_raw(store) as *mut [T; CAPACITY])
        };

        RingSlotStore {
            store,
            max_slot: AtomicU64::new(0),
        }
    }

    #[inline]
    fn index_for_slot(&self, slot: u64) -> usize {
        (slot % (CAPACITY as u64)) as usize
    }
    #[inline]
    pub fn max_slot(&self) -> u64 {
        self.max_slot.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn update_max_slot(&self, slot: u64) {
        self.max_slot.fetch_max(slot, Ordering::Relaxed);
    }

    pub fn get(&self, slot: u64) -> Option<&T> {
        let max_slot_value = self.max_slot();
        if slot > max_slot_value || slot < max_slot_value.saturating_sub(CAPACITY as u64) {
            return None;
        }
        let index = self.index_for_slot(slot);
        let entry = unsafe { self.store.get_unchecked(index) };
        Some(entry)
    }

    pub fn get_unchecked(&self, slot: u64) -> &T {
        let index = self.index_for_slot(slot);
        let entry = unsafe { self.store.get_unchecked(index) };
        entry
    }

    pub fn entry(&mut self, slot: u64) -> Option<&mut T> {
        let index = self.index_for_slot(slot);
        if slot > self.max_slot() {
            return None;
        }
        let entry_ref = unsafe { self.store.get_unchecked_mut(index) };
        Some(entry_ref)
    }
}