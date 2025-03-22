use std::{env, process};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration};
use solana_gossip::contact_info::Protocol;
use solana_sdk::pubkey;
use overcast::repair_peers::RepairPeersManager;
use signal_hook::{consts::SIGINT, flag};

pub fn debug_repair_peers(entrypoint: &str, timeout: u64) {
    let mut manager = RepairPeersManager::new();
    manager.initialize_gossip(entrypoint).unwrap();
    manager.start_refresh_thread(10, 300).unwrap();
    std::thread::sleep(Duration::from_secs(timeout));
    manager.print_repair_peers();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <gossip_entrypoint>", args[0]);
        process::exit(1);
    }

    let gossip_entrypoint = &args[1];
    // debug_repair_peers(gossip_entrypoint, 60);

    let mut manager = RepairPeersManager::new();

    // Initialize gossip
    manager.initialize_gossip(gossip_entrypoint).unwrap();
    // Start background thread to refresh repair peers
    manager.start_refresh_thread(10, 300).unwrap();
    std::thread::sleep(Duration::from_secs(60));
    let my_contact_info = manager.lookup_my_info();
    println!("{:?}", my_contact_info.tvu(Protocol::UDP));
    let contact_info = manager.lookup_info(&pubkey!("C1ocKDYMCm2ooWptMMnpd5VEB2Nx4UMJgRuYofysyzcA"));
    println!("{:?}", contact_info.unwrap().tvu(Protocol::UDP));

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    flag::register(SIGINT, Arc::clone(&running)).expect("Failed to register Ctrl+C handler");

    println!("Press Ctrl+C to exit");

    while r.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    println!("Shutting down...");

    // Clean shutdown
    manager.stop().unwrap();

}
