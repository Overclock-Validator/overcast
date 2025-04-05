use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ahash::{AHashMap, AHashSet, HashMap, HashMapExt};
use crossbeam_channel::{Receiver, Sender};
use solana_ledger::shred::{Shred, ShredType};
use tokio::runtime::Builder;
use crate::queues::get_timestamp_updates_queue;
use crate::types::{RingSlotStore, ShredInfo, DEFER_REPAIR_THRESHOLD};
use tokio_util::time::{delay_queue, DelayQueue};
use tokio_stream::StreamExt;

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
    pub last_fec_set: AtomicU32,
    pub completed: AtomicBool,
}

pub type InProgressSlot = AHashSet<u64>;

#[derive(Clone)]
pub struct SlotMetaStore {
    pub(crate) inner: Arc<RingSlotStore<SlotMetadata>>
}

impl SlotMetaStore {
    pub fn new(metadata_monitor_cons: Receiver<ShredInfo>, repair_monitor_prod: Sender<u64>) -> Self {

        let ring_slot_store = RingSlotStore::<SlotMetadata>::new();
        let meta_store = SlotMetaStore{inner: Arc::new(ring_slot_store)};

        let meta_store_clone = meta_store.clone();
        let (ts_sender, mut ts_receiver ) = get_timestamp_updates_queue();

        // --- Thread 1: Metadata handler ---
        std::thread::spawn(move || {
            while let Ok(shred) = metadata_monitor_cons.recv() {
                if let Ok(deser_shred) = Shred::new_from_serialized_shred(shred.1[0..shred.0].to_vec()) {
                    let slot = deser_shred.slot();
                    match Self::store_shred_meta(&meta_store_clone, &deser_shred) {
                        Ok(_) => {
                            if let Err(e) = ts_sender.blocking_send(slot) {
                                eprintln!("Failed to send timestamp update for slot {}: {}", slot, e);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error storing shred metadata for slot {}: {:?}", slot, e);
                        }
                    }
                }
            }
        });

        // --- Thread 2: Staleness Monitor (using DelayQueue) ---
        let store_arc_for_monitor = meta_store.inner.clone(); // Clone Arc for monitor thread
        let mut ts_receiver_monitor = ts_receiver; // Move receiver ownership

        std::thread::spawn(move || {
            println!("Staleness monitor thread starting...");
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime for monitor thread");

            runtime.block_on(async move {
                let mut delay_queue = DelayQueue::<u64>::new();
                // tack delay queue items to reset the timer
                let mut slot_to_key_map = HashMap::<u64, delay_queue::Key>::new();
                let staleness_duration = Duration::from_millis(DEFER_REPAIR_THRESHOLD);

                loop {
                    tokio::select! {
                        // timestamp updates first
                        biased;

                        // timestamp updates
                        maybe_slot = ts_receiver_monitor.recv() => {
                            if let Some(slot) = maybe_slot {
                                if let Some(key) = slot_to_key_map.get(&slot) {
                                    // reset existing sleeps if new shred is seen
                                    delay_queue.reset(key, staleness_duration);
                                } else {
                                    let key = delay_queue.insert(slot, staleness_duration);
                                    slot_to_key_map.insert(slot, key);
                                }
                            } else {
                                println!("Timestamp channel closed.");
                            }
                        }

                        // handle delay queue expiration
                        maybe_expired = delay_queue.next() => {
                            if let Some(expired) = maybe_expired {
                                let slot = expired.into_inner();
                                slot_to_key_map.remove(&slot);

                                if let Err(e) =  repair_monitor_prod.send(slot) {
                                    eprintln!("Failed to send repair notif for slot {}: {}", slot, e);
                                }
                            }
                        }

                        // graceful shutdown
                        else => {
                            println!("Timestamp channel closed AND DelayQueue stream ended. Monitor shutting down.");
                            break;
                        }
                    }
                }
            });
            println!("Staleness monitor thread finished.");
        }); // end of monitor thread spawn

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

        if shred_type == ShredType::Data {
            if deser_shred.last_in_slot() {
                slot_entry.last_fec_set.store(fec_index, Ordering::Release);
            }
        }

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

        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
        slot_entry.timestamp.store(timestamp, Ordering::Release);

        Ok(())
    }
}

async fn check_and_analyze_slot(
    slot: u64,
    meta_store_arc: Arc<RingSlotStore<SlotMetadata>>,
) {
    let staleness_threshold_millis = 200u64;
    let is_stale;

    // Access SlotMetadata safely
    let slot_entry = meta_store_arc.get_unchecked(slot);
    let stored_epoch_millis = slot_entry.timestamp.load(Ordering::Acquire);
    let current_epoch_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(stored_epoch_millis);

    is_stale = current_epoch_millis.saturating_sub(stored_epoch_millis)
        >= staleness_threshold_millis;

    if is_stale {
        println!(
            "Slot {} confirmed stale (last update ~{}ms ago). Performing analysis.",
            slot,
            current_epoch_millis.saturating_sub(stored_epoch_millis)
        );
        // --- Add your analysis logic here ---
    }

    // --- Cleanup ---
    // No need to remove from AbortHandle map here anymore.
    // The key was removed from slot_to_key_map before this function was called/spawned.
}
