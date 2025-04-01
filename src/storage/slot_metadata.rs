use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use ahash::{AHashMap, AHashSet};
use crossbeam_channel::Receiver;
use solana_ledger::shred::{Shred, ShredType};
use crate::types::{RingSlotStore, ShredInfo};

pub type FecIndex = u32;
pub type Index = u32;
pub type FecMap = AHashMap<FecIndex, Vec<Index>>;
pub type FecInfoMap = AHashMap<FecIndex, FecMeta>;

#[derive(Debug, Default)]
pub struct FecMeta {
    pub(crate) num_data_shreds: u16,
    pub(crate) num_coding_shreds: u16,
}

#[derive(Debug, Default)]
pub struct SlotMetadata {
    pub slot_num: AtomicU64,
    pub fec_data_map: RwLock<FecMap>,
    pub fec_coding_map: RwLock<FecMap>,
    pub fec_meta: RwLock<FecInfoMap>,
    pub timestamp: AtomicU64,
    pub completed: AtomicBool,
}

pub type InProgressSlot = AHashSet<u64>;

#[derive(Clone)]
pub struct SlotMetaStore {
    pub(crate) inner: Arc<RingSlotStore<SlotMetadata>>
}

impl SlotMetaStore {
    pub fn new(repair_monitor_cons: Receiver<ShredInfo>) -> Self {

        let ring_slot_store = RingSlotStore::<SlotMetadata>::new();
        let meta_store = SlotMetaStore{inner: Arc::new(ring_slot_store)};

        let meta_store_clone = meta_store.clone();

        std::thread::spawn(move || {
            while let Ok(shred) = repair_monitor_cons.recv() {
                if let Ok(deser_shred) = Shred::new_from_serialized_shred(shred.1[0..shred.0].to_vec()) {
                    if let Err(e) = Self::store_shred_meta(
                        &meta_store_clone,
                        &deser_shred
                    ) {
                        eprintln!("Error storing shred metadata for slot {}: {:?}", deser_shred.slot(), e);
                    }
                }
            }
        });

        meta_store
    }

    fn store_shred_meta(
        &self,
        deser_shred: &Shred,
    ) -> anyhow::Result<()> {
        let slot = deser_shred.slot();
        let fec_index = deser_shred.fec_set_index();
        let shred_index = deser_shred.index();
        let shred_type = deser_shred.shred_type();
        let slot_entry = self.inner.get_unchecked(slot);
        let stored_slot_num = slot_entry.slot_num.load(Ordering::Acquire);

        if shred_type == ShredType::Code {
            match slot_entry.fec_meta.write() {
                Ok(mut fec_map) => {
                    if !fec_map.contains_key(&fec_index) {
                        if let Shred::ShredCode(coding_shred)  = deser_shred {
                            let coding_header = coding_shred.coding_header();
                            fec_map.insert(fec_index, FecMeta {
                                num_data_shreds: coding_header.num_data_shreds,
                                num_coding_shreds: coding_header.num_coding_shreds
                            });
                        }

                    }
                }
                Err(_) => {}
            }
        }

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


