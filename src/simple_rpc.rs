use std::sync::Arc;
use warp::{Filter, Rejection, Reply};
use serde::{Serialize, Deserialize};
use tokio::runtime::Runtime;
use std::thread;
use std::net::SocketAddr;
use base64::{Engine as _, engine::general_purpose};
use crate::storage::shred_store::ShredStore;
use rayon::prelude::*;
use crate::storage::slot_metadata::SlotMetaStore;
use crate::types::CAPACITY;

#[derive(Deserialize)]
struct SlotShredRequest {
    slot: u64,
    shred_index: u32,
}

#[derive(Deserialize)]
struct SlotRequest {
    slot: u64,
}

#[derive(Serialize)]
struct MaxSlotResponse {
    max_slot: u64,
}

#[derive(Serialize)]
struct ShredResponse {
    slot: u64,
    data: String,
}

#[derive(Serialize)]
struct AllShredsResponse {
    slot: u64,
    shreds: Vec<ShredResponse>,
}

#[derive(Deserialize)]
struct SlotFecRequest {
    slot: u64,
    fec_index: u32,
}

#[derive(Serialize)]
struct FecMetaResponse {
    fec_index: u32,
    num_data_shreds: u16,
    num_coding_shreds: u16,
}

#[derive(Serialize)]
struct SlotFecResponse {
    slot: u64,
    fec_sets: Vec<FecMetaResponse>,
}

#[derive(Serialize)]
struct FecSetDetailsResponse {
    slot: u64,
    fec_index: u32,
    num_data_shreds: u16,
    num_coding_shreds: u16,
    data_indices: Vec<u32>,
    coding_indices: Vec<u32>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub struct SimpleRpcServer {
    store: Arc<ShredStore>,
    meta_store: Arc<SlotMetaStore>,
    runtime: Option<Runtime>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl SimpleRpcServer {
    pub fn new(store: ShredStore, meta_store: SlotMetaStore) -> Self {
        SimpleRpcServer {
            store: Arc::new(store),
            meta_store: Arc::new(meta_store),
            runtime: None,
            thread_handle: None,
        }
    }

    pub fn start(&mut self, addr: SocketAddr) -> anyhow::Result<()> {
        let store_clone = self.store.clone();
        let meta_store_clone = self.meta_store.clone();

        let handle = thread::spawn(move || {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");

            rt.block_on(async {
                let max_slot_route = warp::path!("max_slot")
                    .and(warp::get())
                    .and(with_store(store_clone.clone()))
                    .and_then(handle_max_slot);

                let slot_shred_route = warp::path!("slot" / "shred")
                    .and(warp::get())
                    .and(warp::query::<SlotShredRequest>())
                    .and(with_store(store_clone.clone()))
                    .and_then(handle_slot_shred);

                let all_shreds_route = warp::path!("slot" / "all_shreds")
                    .and(warp::get())
                    .and(warp::query::<SlotRequest>())
                    .and(with_store(store_clone.clone()))
                    .and_then(handle_all_shreds);

                let slot_fec_meta_route = warp::path!("slot" / "fec_meta")
                    .and(warp::get())
                    .and(warp::query::<SlotRequest>())
                    .and(with_meta_store(meta_store_clone.clone()))
                    .and_then(handle_slot_fec_meta);

                let fec_set_details_route = warp::path!("slot" / "fec_set")
                    .and(warp::get())
                    .and(warp::query::<SlotFecRequest>())
                    .and(with_meta_store(meta_store_clone.clone()))
                    .and_then(handle_fec_set_details);

                let meta_max_slot_route = warp::path!("meta" / "max_slot")
                    .and(warp::get())
                    .and(with_meta_store(meta_store_clone.clone()))
                    .and_then(handle_meta_max_slot);

                let routes = max_slot_route
                    .or(slot_shred_route)
                    .or(all_shreds_route)
                    .or(slot_fec_meta_route)
                    .or(fec_set_details_route)
                    .or(meta_max_slot_route)
                    .with(warp::cors().allow_any_origin());

                println!("RPC server started on {}", addr);

                warp::serve(routes).run(addr).await;
            });
        });

        self.thread_handle = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.thread_handle.take() {
            println!("RPC server shutdown initiated (will complete on process exit)");
            std::mem::forget(handle);
        }
    }
}
fn with_meta_store(
    store: Arc<SlotMetaStore>,
) -> impl Filter<Extract = (Arc<SlotMetaStore>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || store.clone())
}
fn with_store(
    store: Arc<ShredStore>,
) -> impl Filter<Extract = (Arc<ShredStore>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || store.clone())
}

async fn handle_max_slot(
    store: Arc<ShredStore>,
) -> Result<impl Reply, Rejection> {
    let max_slot = store.max_slot();

    let response = MaxSlotResponse {
        max_slot,
    };

    Ok(warp::reply::json(&response))
}

async fn handle_slot_shred(
    query: SlotShredRequest,
    store: Arc<ShredStore>,
) -> Result<impl Reply, Rejection> {
    let slot = query.slot;
    let shred_index = query.shred_index;
    let shreds = store.get_shred_num(slot, shred_index);
    match shreds.0 {
        Some((length, data)) => {
            // Base64 encode the shred data for transmission
            let encoded = general_purpose::STANDARD.encode(&data[0..length]);

            let response = ShredResponse {
                slot,
                data: encoded,
            };

            Ok(warp::reply::json(&response))
        },
        None => {
            let response = ErrorResponse {
                error: format!("Shred not found for slot {} index {}", slot, shred_index),
            };

            Ok(warp::reply::json(&response))
        }
    }
}

async fn handle_all_shreds(
    query: SlotRequest,
    store: Arc<ShredStore>,
) -> Result<impl Reply, Rejection> {
    let slot = query.slot;
    let max_slot = store.max_slot();

    if slot > max_slot || slot < max_slot.saturating_sub(CAPACITY as u64) {
        let response = ErrorResponse {
            error: format!("Slot {} not in valid range (max_slot={})", slot, max_slot),
        };

        return Ok(warp::reply::json(&response));
    }

    let mut shreds = Vec::new();
    let stored_shreds = store.get_slot_shreds(slot);

    for shred_info in stored_shreds {
        let encoded = general_purpose::STANDARD.encode(&shred_info.1[0..shred_info.0]);
        shreds.push(ShredResponse {
            slot,
            data: encoded,
        });
    }


    if shreds.is_empty() {
        let response = ErrorResponse {
            error: format!("No shreds found for slot {}", slot),
        };

        return Ok(warp::reply::json(&response));
    }

    let response = AllShredsResponse {
        slot,
        shreds,
    };

    Ok(warp::reply::json(&response))
}

// Handle max slot for meta store
async fn handle_meta_max_slot(
    meta_store: Arc<SlotMetaStore>,
) -> Result<impl Reply, Rejection> {
    let max_slot = meta_store.inner.max_slot();

    let response = MaxSlotResponse {
        max_slot,
    };

    Ok(warp::reply::json(&response))
}

// Handle FEC metadata for a slot
async fn handle_slot_fec_meta(
    query: SlotRequest,
    meta_store: Arc<SlotMetaStore>,
) -> Result<impl Reply, Rejection> {
    let slot = query.slot;
    let max_slot = meta_store.inner.max_slot();

    // Check if slot is in valid range
    if slot > max_slot || slot < max_slot.saturating_sub(CAPACITY as u64) {
        let response = ErrorResponse {
            error: format!("Slot {} not in valid range (max_slot={})", slot, max_slot),
        };
        return Ok(warp::reply::json(&response));
    }

    let slot_entry = meta_store.inner.get_unchecked(slot);
    let stored_slot_num = slot_entry.slot_num.load(std::sync::atomic::Ordering::Relaxed);

    // Check if slot matches
    if stored_slot_num != slot {
        let response = ErrorResponse {
            error: format!("Slot mismatch: expected {}, found {}", slot, stored_slot_num),
        };
        return Ok(warp::reply::json(&response));
    }

    // Read FEC metadata
    let mut fec_sets = Vec::new();
    if let Ok(fec_meta_guard) = slot_entry.fec_meta.read() {
        for (&fec_index, meta) in fec_meta_guard.iter() {
            fec_sets.push(FecMetaResponse {
                fec_index,
                num_data_shreds: meta.num_data_shreds,
                num_coding_shreds: meta.num_coding_shreds,
            });
        }
    }

    if fec_sets.is_empty() {
        let response = ErrorResponse {
            error: format!("No FEC metadata found for slot {}", slot),
        };
        return Ok(warp::reply::json(&response));
    }

    let response = SlotFecResponse {
        slot,
        fec_sets,
    };

    Ok(warp::reply::json(&response))
}

// Handle details for a specific FEC set
async fn handle_fec_set_details(
    query: SlotFecRequest,
    meta_store: Arc<SlotMetaStore>,
) -> Result<impl Reply, Rejection> {
    let slot = query.slot;
    let fec_index = query.fec_index;
    let max_slot = meta_store.inner.max_slot();

    // Check if slot is in valid range
    if slot > max_slot || slot < max_slot.saturating_sub(CAPACITY as u64) {
        let response = ErrorResponse {
            error: format!("Slot {} not in valid range (max_slot={})", slot, max_slot),
        };
        return Ok(warp::reply::json(&response));
    }

    let slot_entry = meta_store.inner.get_unchecked(slot);
    let stored_slot_num = slot_entry.slot_num.load(std::sync::atomic::Ordering::Relaxed);

    // Check if slot matches
    if stored_slot_num != slot {
        let response = ErrorResponse {
            error: format!("Slot mismatch: expected {}, found {}", slot, stored_slot_num),
        };
        return Ok(warp::reply::json(&response));
    }

    // Get FEC metadata
    let mut num_data_shreds = 0;
    let mut num_coding_shreds = 0;

    if let Ok(fec_meta_guard) = slot_entry.fec_meta.read() {
        if let Some(meta) = fec_meta_guard.get(&fec_index) {
            num_data_shreds = meta.num_data_shreds;
            num_coding_shreds = meta.num_coding_shreds;
        } else {
            let response = ErrorResponse {
                error: format!("FEC set {} not found for slot {}", fec_index, slot),
            };
            return Ok(warp::reply::json(&response));
        }
    } else {
        let response = ErrorResponse {
            error: format!("Failed to acquire read lock for FEC metadata"),
        };
        return Ok(warp::reply::json(&response));
    }

    // Get data shred indices
    let mut data_indices = Vec::new();
    if let Ok(data_map_guard) = slot_entry.fec_data_map.read() {
        if let Some(indices) = data_map_guard.get(&fec_index) {
            data_indices = indices.clone();
        }
    } else {
        let response = ErrorResponse {
            error: format!("Failed to acquire read lock for data map"),
        };
        return Ok(warp::reply::json(&response));
    }

    // Get coding shred indices
    let mut coding_indices = Vec::new();
    if let Ok(coding_map_guard) = slot_entry.fec_coding_map.read() {
        if let Some(indices) = coding_map_guard.get(&fec_index) {
            coding_indices = indices.clone();
        }
    } else {
        let response = ErrorResponse {
            error: format!("Failed to acquire read lock for coding map"),
        };
        return Ok(warp::reply::json(&response));
    }

    let response = FecSetDetailsResponse {
        slot,
        fec_index,
        num_data_shreds,
        num_coding_shreds,
        data_indices,
        coding_indices,
    };

    Ok(warp::reply::json(&response))
}