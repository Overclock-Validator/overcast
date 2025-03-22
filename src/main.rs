use std::{env, process};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use solana_gossip::contact_info::Protocol;
use signal_hook::{consts::SIGINT, flag};

mod gossip;
mod repair_manager;

use gossip::GossipManager;
use repair_manager::RepairPeersManager;

pub fn debug_repair_peers(entrypoint: &str, timeout: u64) {
    let mut gossip_manager = GossipManager::new();
    gossip_manager.initialize(entrypoint).unwrap();

    let mut repair_manager = RepairPeersManager::new(&gossip_manager);
    repair_manager.start_refresh_thread(10, 300).unwrap();
    std::thread::sleep(Duration::from_secs(timeout));
    repair_manager.print_repair_peers();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <gossip_entrypoint>", args[0]);
        process::exit(1);
    }

    let gossip_entrypoint = &args[1];
    // debug_repair_peers(gossip_entrypoint, 60);

    let mut gossip_manager = GossipManager::new();
    gossip_manager.initialize(gossip_entrypoint).unwrap();

    let mut repair_manager = RepairPeersManager::new(&gossip_manager);
    repair_manager.start_refresh_thread(10, 300).unwrap();

    let my_contact_info = gossip_manager.lookup_my_info();
    println!("me: {:?}", my_contact_info.tvu(Protocol::UDP));

    let sigint_recv = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&sigint_recv)).expect("Failed to register signal handler");

    println!("Press Ctrl+C to exit");
    while !sigint_recv.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("Received Ctrl+C, exiting");
    println!("Shutting down...");

    repair_manager.stop().unwrap();
    // Explicit drop because borrow checker
    drop(repair_manager);
    gossip_manager.stop().unwrap();
}