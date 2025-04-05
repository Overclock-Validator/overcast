use std::{env, process};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use solana_gossip::contact_info::Protocol;
use signal_hook::{consts::SIGINT, flag};

use overcast::gossip::GossipManager;
use overcast::queues::{get_metadata_queue, get_repair_queue, get_storage_queue};
use overcast::storage::shred_store::ShredStore;
use overcast::simple_rpc::SimpleRpcServer;
use overcast::turbine_manager::TurbineManager;
use overcast::repair::repair_manager::RepairPeersManager;
use overcast::storage::slot_metadata::SlotMetaStore;

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
    let (metadata_monitor_prod, metadata_monitor_cons) = get_metadata_queue();
    let (repair_monitor_prod, repair_monitor_cons) = get_repair_queue();

    let gossip_entrypoint = &args[1];
    // debug_repair_peers(gossip_entrypoint, 60);

    let mut gossip_manager = GossipManager::new();
    gossip_manager.initialize(gossip_entrypoint).unwrap();

    let mut repair_manager = RepairPeersManager::new(&gossip_manager);
    repair_manager.start_refresh_thread(10, 300).unwrap();

    let (store_send, store_recv) = get_storage_queue();
    let store = ShredStore::new(store_recv);

    let my_contact_info = gossip_manager.lookup_my_info();
    let my_tvu_addr =  my_contact_info.tvu(Protocol::UDP).unwrap();
    println!("me: {:?}", my_tvu_addr);

    let meta_store = SlotMetaStore::new(metadata_monitor_cons, repair_monitor_prod);

    let mut turbine_manager = TurbineManager::new(my_tvu_addr).unwrap();
    turbine_manager.run(store_send, metadata_monitor_prod);

    let rpc_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
    let mut rpc_server = SimpleRpcServer::new(store.clone(), meta_store.clone());
    rpc_server.start(rpc_addr).unwrap();

    let sigint_recv = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&sigint_recv)).expect("Failed to register signal handler");

    println!("Press Ctrl+C to exit");
    while !sigint_recv.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("Received Ctrl+C, exiting");
    println!("Shutting down...");

    rpc_server.stop();
    repair_manager.stop().unwrap();
    // Explicit drop because borrow checker (:
    drop(repair_manager);
    gossip_manager.stop().unwrap();

}