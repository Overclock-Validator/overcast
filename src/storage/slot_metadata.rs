use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use ahash::AHashMap;
use crossbeam_channel::Receiver;
use solana_ledger::shred::{Shred, ShredType};
use crate::types::{RingSlotStore, ShredInfo, SHRED_SIZE};

pub type FecIndex = u32;
pub type Index = u32;
pub type FecMap = AHashMap<FecIndex, Vec<Index>>;

#[derive(Debug, Default)]
pub struct SlotMetadata {
    slot_num: AtomicU64,
    fec_data_map: RwLock<FecMap>,
    fec_coding_map: RwLock<FecMap>,
    timestamp: AtomicU64,
    completed: AtomicBool,
}

#[derive(Clone)]
pub struct SlotMetaStore {
    inner: Arc<RingSlotStore<SlotMetadata>>
}

impl SlotMetaStore {
    pub fn new(rx: Receiver<ShredInfo>) -> Self {
        let ring_slot_store = RingSlotStore::<SlotMetadata>::new();
        let meta_store = SlotMetaStore{inner: Arc::new(ring_slot_store)};

        let meta_store_clone = meta_store.clone();

        std::thread::spawn(move || {
            while let Ok(shred) = rx.recv() {
                if let Ok(deser_shred) = Shred::new_from_serialized_shred(shred.1[0..shred.0].to_vec()) {
                    let slot = deser_shred.slot();
                    let fec_index = deser_shred.fec_set_index();
                    let shred_index = deser_shred.index();

                    if let Err(e) = Self::store_shred_meta(
                        &meta_store_clone,
                        slot,
                        fec_index,
                        shred_index,
                        deser_shred.shred_type()
                    ) {
                        eprintln!("Error storing shred metadata for slot {}: {:?}", slot, e);
                    }
                }
            }
        });

        meta_store
    }

    fn store_shred_meta(
        &self,
        slot: u64,
        fec_index: u32,
        shred_index: u32,
        shred_type: ShredType,
    ) -> anyhow::Result<()> {
        let slot_entry = self.inner.get_unchecked(slot);
        let stored_slot_num = slot_entry.slot_num.load(Ordering::Acquire);

        if slot >= stored_slot_num {
            if slot > stored_slot_num {
                match slot_entry.fec_data_map.write() {
                    Ok(mut data_map) => {
                        data_map.clear();
                    },
                    Err(e) => return Err(anyhow::anyhow!("Failed to acquire data write lock: {}", e)),
                }

                match slot_entry.fec_coding_map.write() {
                    Ok(mut coding_map) => {
                        coding_map.clear();
                    },
                    Err(e) => return Err(anyhow::anyhow!("Failed to acquire data write lock: {}", e)),
                }

                slot_entry.slot_num.store(slot, Ordering::Release);
            }

            // Handle shred insertion similarly
            match shred_type {
                ShredType::Data => {
                    match slot_entry.fec_data_map.write() {
                        Ok(mut data_map) => {
                            data_map.entry(fec_index).or_insert_with(Vec::new).push(shred_index);
                        },
                        Err(e) => return Err(anyhow::anyhow!("Failed to acquire data write lock for insertion: {}", e)),
                    }
                },
                ShredType::Code => {
                    match slot_entry.fec_coding_map.write() {
                        Ok(mut code_map) => {
                            code_map.entry(fec_index).or_insert_with(Vec::new).push(shred_index);
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
}

