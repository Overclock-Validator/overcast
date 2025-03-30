use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use ahash::AHashMap;
use crossbeam_channel::Receiver;
use solana_ledger::shred::{Shred, ShredType};
use crate::types::{ShredInfo, CAPACITY, SHRED_SIZE, RingSlotStore};

pub type SlotMap = AHashMap<u32, ShredInfo>;
pub type SlotInfo = (AtomicU64, (RwLock<SlotMap>, RwLock<SlotMap>));

#[derive(Clone)]
pub struct ShredStore {
    inner: Arc<RingSlotStore<SlotInfo>>
}

impl ShredStore {
    pub fn new(rx: Receiver<ShredInfo>) -> Self {
        let ring_slot_store = RingSlotStore::<SlotInfo>::new();
        let shred_store = ShredStore{inner:Arc::new(ring_slot_store)};

        let shred_store_clone = shred_store.clone();

        std::thread::spawn(move || {
            while let Ok(shred) = rx.recv() {
                if let Ok(deser_shred) = Shred::new_from_serialized_shred(shred.1[0..shred.0].to_vec()) {
                    let slot = deser_shred.slot();
                    let index = deser_shred.index();

                    if let Err(e) = Self::store_shred_internal(
                        &shred_store_clone,
                        slot,
                        index,
                        shred.1,
                        shred.0,
                        deser_shred.shred_type()
                    ) {
                        eprintln!("Error storing shred for slot {}: {:?}", slot, e);
                    }
                }
            }
        });

        shred_store
    }

    fn store_shred_internal(
        &self,
        slot: u64,
        shred_index: u32,
        shred: [u8; SHRED_SIZE],
        shred_len: usize,
        shred_type: ShredType,
    ) -> anyhow::Result<()> {
        let slot_entry = self.inner.get_unchecked(slot);
        let stored_slot_num = slot_entry.0.load(Ordering::Acquire);

        if slot >= stored_slot_num {
            if slot > stored_slot_num {
                // Don't use ? for write() - handle the Result explicitly
                match slot_entry.1.0.write() {
                    Ok(mut data_shred_store) => {
                        data_shred_store.clear();
                        // Guard is dropped here automatically
                    },
                    Err(e) => return Err(anyhow::anyhow!("Failed to acquire data write lock: {}", e)),
                }

                match slot_entry.1.1.write() {
                    Ok(mut coding_shred_store) => {
                        coding_shred_store.clear();
                        // Guard is dropped here automatically
                    },
                    Err(e) => return Err(anyhow::anyhow!("Failed to acquire coding write lock: {}", e)),
                }

                slot_entry.0.store(slot, Ordering::Release);
            }

            // Handle shred insertion similarly
            match shred_type {
                ShredType::Data => {
                    match slot_entry.1.0.write() {
                        Ok(mut data_map) => {
                            data_map.insert(shred_index, (shred_len, shred));
                        },
                        Err(e) => return Err(anyhow::anyhow!("Failed to acquire data write lock for insertion: {}", e)),
                    }
                },
                ShredType::Code => {
                    match slot_entry.1.1.write() {
                        Ok(mut code_map) => {
                            code_map.insert(shred_index, (shred_len, shred));
                        },
                        Err(e) => return Err(anyhow::anyhow!("Failed to acquire coding write lock for insertion: {}", e)),
                    }
                },
            }
        }

        let current_max = self.inner.max_slot();
        if slot > current_max {
            self.inner.update_max_slot(slot)
        }

        Ok(())
    }

    fn get_shred_internal(
        &self,
        slot_num: u64,
        shred_num: u32
    ) -> (Option<ShredInfo>, Option<ShredInfo>) {
        let max_slot_value = self.inner.max_slot();

        if slot_num > max_slot_value || slot_num < max_slot_value.saturating_sub(CAPACITY as u64) {
            println!("DEBUG: Slot {} not in range (max_slot={})", slot_num, max_slot_value);
            return (None,None);
        }

        let slot_entry = self.inner.get_unchecked(slot_num);

        if slot_entry.0.load(Ordering::Relaxed) == slot_num {
            // println!("DEBUG: Found slot {} in index {}, checking for shred {}",
            //          slot_num, index, shred_num);
            let data_map = slot_entry.1.0.read().unwrap();
            let code_map = slot_entry.1.1.read().unwrap();
            (data_map.get(&shred_num).cloned(), code_map.get(&shred_num).cloned() )
        } else {
            // println!("DEBUG: Slot mismatch at index {}: expected {}, found {}",
            //          index, slot_num, slot_entry.0);
            (None, None)
        }
    }

    pub fn get_shred_num(&self, slot_num: u64, shred_num: u32) -> (Option<ShredInfo>, Option<ShredInfo>) {
        self.get_shred_internal(slot_num, shred_num)
    }

    pub fn get_slot_shreds(&self, slot_num: u64) -> Vec<ShredInfo> {
        let max_slot_value = self.inner.max_slot();

        if slot_num > max_slot_value || slot_num < max_slot_value.saturating_sub(CAPACITY as u64) {
            return Vec::new();
        }

        let slot_entry = self.inner.get_unchecked(slot_num);

        if slot_entry.0.load(Ordering::Relaxed) != slot_num {
            return Vec::new();
        }

        let data_map = slot_entry.1.0.read().unwrap();
        let code_map = slot_entry.1.1.read().unwrap();

        let mut result: Vec<ShredInfo> = data_map
            .values()
            .cloned()
            .collect();

        result.extend(
            code_map
                .values()
                .cloned()
        );

        result
    }

    pub fn max_slot(&self) -> u64 {
        self.inner.max_slot()
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

        assert!(retrieved.0.is_some(), "Expected shred not found in slot {}", slot);
        let (ret_len, ret_shred) = retrieved.0.unwrap();
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

        assert!(retrieved1.0.is_some(), "Shred with index 10 not found");
        assert!(retrieved2.0.is_some(), "Shred with index 20 not found");

        let (ret_len1, ret_shred1) = retrieved1.0.unwrap();
        let (ret_len2, ret_shred2) = retrieved2.0.unwrap();

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

        assert!(retrieved_new.0.is_some(), "Newer shred not found");
        assert!(retrieved_old.0.is_some(), "Older shred not found");
    }
}