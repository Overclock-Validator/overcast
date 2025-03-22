use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use solana_sdk::packet;
use ahash::AHashMap;
use crossbeam_channel::Receiver;
use solana_ledger::shred::{Shred};
use crate::types::ShredType;

pub(crate) const CAPACITY: usize = 1 << 12;
const SHRED_SIZE: usize = packet::PACKET_DATA_SIZE;

pub type SlotMap = AHashMap<u32, ShredType>;

#[derive(Clone)]
pub struct ShredStore {
    slots: Arc<Box<[RwLock<(u64, SlotMap)>; CAPACITY]>>,
    max_slot: Arc<AtomicU64>,
}

impl ShredStore {
    pub fn new(rx: Receiver<ShredType>) -> Self {
        let mut slots_vec = Vec::with_capacity(CAPACITY);
        for _ in 0..CAPACITY {
            slots_vec.push(RwLock::new((0u64, AHashMap::new())));
        }

        let slots_array: Box<[RwLock<(u64, SlotMap)>; CAPACITY]> =
            slots_vec.into_boxed_slice().try_into().unwrap_or_else(|_| panic!("Failed to create slots array"));

        let slots = Arc::new(slots_array);
        let max_slot = Arc::new(AtomicU64::new(0));

        let store = Self {
            slots: slots.clone(),
            max_slot: max_slot.clone(),
        };

        let thread_slots = slots.clone();
        let thread_max_slot = max_slot.clone();

        std::thread::spawn(move || {
            while let Ok(shred) = rx.recv() {
                if let Ok(deser_shred) = Shred::new_from_serialized_shred(shred.1[0..shred.0].to_vec()) {
                    let slot = deser_shred.slot();
                    let index = deser_shred.index();

                    if let Err(e) = Self::store_shred_internal(
                        &thread_slots,
                        &thread_max_slot,
                        slot,
                        index,
                        shred.1,
                        shred.0
                    ) {
                        eprintln!("Error storing shred for slot {}: {:?}", slot, e);
                    }
                }
            }
        });

        store
    }

    fn store_shred_internal(
        slots: &Arc<Box<[RwLock<(u64, SlotMap)>; CAPACITY]>>,
        max_slot: &Arc<AtomicU64>,
        slot: u64,
        shred_index: u32,
        shred: [u8; SHRED_SIZE],
        shred_len: usize
    ) -> anyhow::Result<()> {
        let index = (slot % (CAPACITY as u64)) as usize;

        // lock the specific slot
        let mut slot_entry = slots[index].write().unwrap();
        let stored_slot_num = slot_entry.0;

        if slot > stored_slot_num {
            slot_entry.1 = AHashMap::new();
            slot_entry.0 = slot;
            slot_entry.1.insert(shred_index, (shred_len, shred));
        } else if slot == stored_slot_num {
            slot_entry.1.insert(shred_index, (shred_len, shred));
        }

        drop(slot_entry);

        // atomic update for max slot
        let current_max = max_slot.load(Ordering::Relaxed);
        if slot > current_max {
            max_slot.store(slot, Ordering::Relaxed);
        }

        // let x = Self::get_shred_internal(slots, max_slot, slot, shred_index);
        // println!("DEBUG: After storing - slot={}, index={}, exists={}",
        //          slot, shred_index, x.is_some());

        Ok(())
    }

    fn get_shred_internal(
        slots: &Arc<Box<[RwLock<(u64, SlotMap)>; CAPACITY]>>,
        max_slot: &Arc<AtomicU64>,
        slot_num: u64,
        shred_num: u32
    ) -> Option<ShredType> {
        let max_slot_value = max_slot.load(Ordering::Relaxed);

        if slot_num > max_slot_value || slot_num < max_slot_value.saturating_sub(CAPACITY as u64) {
            println!("DEBUG: Slot {} not in range (max_slot={})", slot_num, max_slot_value);
            return None;
        }

        let index = (slot_num % (CAPACITY as u64)) as usize;

        let slot_entry = slots[index].read().unwrap();

        if slot_entry.0 == slot_num {
            // println!("DEBUG: Found slot {} in index {}, checking for shred {}",
            //          slot_num, index, shred_num);
            slot_entry.1.get(&shred_num).cloned()
        } else {
            // println!("DEBUG: Slot mismatch at index {}: expected {}, found {}",
            //          index, slot_num, slot_entry.0);
            None
        }
    }

    pub fn get_shred_num(&self, slot_num: u64, shred_num: u32) -> Option<ShredType> {
        Self::get_shred_internal(&self.slots, &self.max_slot, slot_num, shred_num)
    }

    pub fn max_slot(&self) -> u64 {
        self.max_slot.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;
    use super::*;
    use solana_ledger::shred::{Shred, ShredFlags};
    use crate::queues::get_storage_queue;

    fn create_dummy_shred(slot: u64, shred_index: u32) -> ([u8; SHRED_SIZE], usize) {
        let data = vec![1u8; 100];
        let flags = ShredFlags::DATA_COMPLETE_SHRED;
        let reference_tick = 0;
        let version = 1;
        let fec_set_index = 0;

        let shred = Shred::new_from_data(
            slot,
            shred_index,
            10,         // parent_offset
            &data,
            flags,
            reference_tick,
            version,
            fec_set_index,
        );

        let payload = shred.into_payload();
        let shred_len = payload.len();
        let mut arr = [0u8; SHRED_SIZE];

        arr[..shred_len].copy_from_slice(&payload);
        (arr, shred_len)
    }

    #[test]
    fn test_store_new_slot() {
        let storage_queue = get_storage_queue();
        let store = ShredStore::new(storage_queue.1);

        let slot = 100;
        let shred_index = 5;
        let (shred_bytes, shred_len) = create_dummy_shred(slot, shred_index);

        storage_queue.0.send((shred_len, shred_bytes)).unwrap();

        // sleep because async writes
        thread::sleep(Duration::from_millis(100));

        println!("\nTEST: Retrieving shred at slot={}, index={}", slot, shred_index);
        let retrieved = store.get_shred_num(slot, shred_index);

        assert!(retrieved.is_some(), "Expected shred not found in slot {}", slot);
        let (ret_len, ret_shred) = retrieved.unwrap();
        assert_eq!(ret_len, shred_len);
        assert_eq!(ret_shred, shred_bytes);
    }

    #[test]
    fn test_store_multiple_shreds_same_slot() {
        let storage_queue = get_storage_queue();
        let store = ShredStore::new(storage_queue.1);

        let slot = 200;

        let (shred_bytes1, shred_len1) = create_dummy_shred(slot, 10);
        let (shred_bytes2, shred_len2) = create_dummy_shred(slot, 20);

                storage_queue.0.send((shred_len1, shred_bytes1)).unwrap();
        storage_queue.0.send((shred_len2, shred_bytes2)).unwrap();

        // sleep because asycn writes
        thread::sleep(Duration::from_millis(100));

        let retrieved1 = store.get_shred_num(slot, 10);
        let retrieved2 = store.get_shred_num(slot, 20);

        assert!(retrieved1.is_some(), "Shred with index 10 not found");
        assert!(retrieved2.is_some(), "Shred with index 20 not found");

        let (ret_len1, ret_shred1) = retrieved1.unwrap();
        let (ret_len2, ret_shred2) = retrieved2.unwrap();

        assert_eq!(ret_len1, shred_len1);
        assert_eq!(ret_shred1, shred_bytes1);
        assert_eq!(ret_len2, shred_len2);
        assert_eq!(ret_shred2, shred_bytes2);
    }

    #[test]
    fn test_store_out_of_order() {
        let storage_queue = get_storage_queue();
        let store = ShredStore::new(storage_queue.1);

        let slot_new = 300;
        let shred_index_new = 15;
        let (shred_bytes_new, shred_len_new) = create_dummy_shred(slot_new, shred_index_new);
        storage_queue.0.send((shred_len_new, shred_bytes_new)).unwrap();

        // sleep because async writes
        thread::sleep(Duration::from_millis(50));

        assert_eq!(store.max_slot(), slot_new);

        // older shred
        let slot_old = 299;
        let shred_index_old = 25;
        let (shred_bytes_old, shred_len_old) = create_dummy_shred(slot_old, shred_index_old);
        storage_queue.0.send((shred_len_old, shred_bytes_old)).unwrap();

        // sleep because async writes
        thread::sleep(Duration::from_millis(50));

        assert_eq!(store.max_slot(), slot_new);

        let retrieved_new = store.get_shred_num(slot_new, shred_index_new);
        let retrieved_old = store.get_shred_num(slot_old, shred_index_old);

        assert!(retrieved_new.is_some(), "Newer shred not found");
        assert!(retrieved_old.is_some(), "Older shred not found");
    }
}