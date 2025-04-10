use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender};
use crate::storage::slot_metadata::SlotMetaStore;
use crate::types::ShredInfo;

pub struct RepairHandler {
    meta_store: SlotMetaStore,
    repair_notif_recv: Receiver<u64>,
    storage_sender: Sender<ShredInfo>,
}

impl RepairHandler {
    pub fn new(meta_store: SlotMetaStore, repair_notif_recv: Receiver<u64>, storage_sender: Sender<ShredInfo> ) -> Self {
        RepairHandler {
            meta_store,
            repair_notif_recv,
            storage_sender,
        }
    }

    pub fn start(self) {
        let thread_handle = std::thread::spawn(move || {
            loop {
                if let Ok(slot) = self.repair_notif_recv.recv() {
                    let mut missing_data_shreds = vec![];
                    let mut missing_coding_shreds = vec![];
                    let slot_metadata = self.meta_store.inner.get_unchecked(slot);
                    // check data shreds
                    if let Ok(fec_data_map) = slot_metadata.fec_data_map.read() {
                        for (k,v) in fec_data_map.iter() {
                            if let Ok(fec_meta) = slot_metadata.fec_meta.read() {
                                fec_meta.get(k);
                            }
                            // if
                        }
                    }
                    // check repair shreds

                    // self.meta_store.inner.
                }
            }

        });
        if let Err(e) = thread_handle.join() {
            eprintln!("Error joining thread {:?}", e);
        };
    }
}
pub fn check_slot_for_repair() {}

pub fn mark_slot_completed() {}

pub fn send_repair() {}

pub fn handle_repair() {}


