use std::sync::Arc;
use warp::{Filter, Rejection, Reply};
use serde::{Serialize, Deserialize};
use tokio::runtime::Runtime;
use std::thread;
use std::net::SocketAddr;
use base64::{Engine as _, engine::general_purpose};
use solana_ledger::shred::{ReedSolomonCache, Shred, Shredder};
use crate::storage::shred_store::ShredStore;
use rayon::prelude::*;
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

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub struct SimpleRpcServer {
    store: Arc<ShredStore>,
    runtime: Option<Runtime>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl SimpleRpcServer {
    pub fn new(store: ShredStore) -> Self {
        SimpleRpcServer {
            store: Arc::new(store),
            runtime: None,
            thread_handle: None,
        }
    }

    pub fn start(&mut self, addr: SocketAddr) -> anyhow::Result<()> {
        let store_clone = self.store.clone();

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

                let routes = max_slot_route
                    .or(slot_shred_route)
                    .or(all_shreds_route)
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
