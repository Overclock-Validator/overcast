use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use solana_gossip::cluster_info::ClusterInfo;
use solana_gossip::contact_info::{ContactInfo, Protocol};
use solana_sdk::pubkey::Pubkey;

use crate::gossip::GossipManager;

struct RepairPeerInfo {
    socket_addr: SocketAddr,
    last_seen: SystemTime,
}

pub struct RepairPeersManager<'a> {
    repair_peers: Arc<Mutex<HashMap<IpAddr, RepairPeerInfo>>>,
    exit: Arc<AtomicBool>,
    refresh_thread: Option<thread::JoinHandle<()>>,
    gossip_manager: &'a GossipManager,
}

impl<'a> RepairPeersManager<'a> {
    pub fn new(gossip_manager: &'a GossipManager) -> Self {
        RepairPeersManager {
            repair_peers: Arc::new(Mutex::new(HashMap::new())),
            exit: Arc::new(AtomicBool::new(false)),
            refresh_thread: None,
            gossip_manager,
        }
    }

    pub fn start_refresh_thread(&mut self, refresh_interval: u64, stale_threshold: u64) -> Result<(), Box<dyn std::error::Error>> {
        let repair_peers = Arc::clone(&self.repair_peers);
        let exit = Arc::clone(&self.exit);
        let cluster_info = self.gossip_manager.get_cluster_info()
            .ok_or("Cluster info not initialized")?;

        let thread = thread::spawn(move || {
            refresh_repair_peers(refresh_interval, stale_threshold, repair_peers, exit, cluster_info);
        });

        self.refresh_thread = Some(thread);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.exit.store(true, Ordering::Relaxed);

        if let Some(thread) = self.refresh_thread.take() {
            thread.join().map_err(|_| "Failed to join refresh thread")?;
        }

        Ok(())
    }

    pub fn get_repair_peers(&self) -> Vec<SocketAddr> {
        let peers_lock = self.repair_peers.lock().unwrap();
        peers_lock.values().map(|info| info.socket_addr).collect()
    }

    pub fn print_repair_peers(&self) {
        let peers_lock = self.repair_peers.lock().unwrap();
        println!("Current repair peers:");
        for (ip, info) in peers_lock.iter() {
            let duration = SystemTime::now().duration_since(info.last_seen).unwrap_or(Duration::from_secs(0));
            println!("IP: {}, Address: {:?}, Last seen: {} seconds ago", ip, info.socket_addr, duration.as_secs());
        }
        println!("{} repair peers found", peers_lock.len());
    }
}

fn refresh_repair_peers(
    refresh_interval: u64,
    stale_threshold: u64,
    repair_peers: Arc<Mutex<HashMap<IpAddr, RepairPeerInfo>>>,
    exit: Arc<AtomicBool>,
    cluster_info: Arc<ClusterInfo>
) {
    while !exit.load(Ordering::Relaxed) {
        let peers = cluster_info.all_peers();
        let current_time = SystemTime::now();

        {
            let mut peers_lock = repair_peers.lock().unwrap();

            // stale cleanup
            peers_lock.retain(|_, info| {
                if let Ok(duration) = current_time.duration_since(info.last_seen) {
                    duration.as_secs() < stale_threshold
                } else {
                    true
                }
            });

            // update
            for (peer, _) in peers {
                if let Some(peer_repair_addr) = peer.serve_repair(Protocol::UDP) {
                    let ip = peer_repair_addr.ip();
                    peers_lock.insert(ip, RepairPeerInfo {
                        socket_addr: peer_repair_addr,
                        last_seen: current_time,
                    });
                }
            }

            println!("Refreshed repair peers. Current count: {}", peers_lock.len());
        }

        // sleep
        let mut remaining_sleep = refresh_interval;
        while remaining_sleep > 0 && !exit.load(Ordering::Relaxed) {
            // check exit 5 seconds
            let sleep_chunk = std::cmp::min(remaining_sleep, 5);
            std::thread::sleep(Duration::from_secs(sleep_chunk));
            remaining_sleep -= sleep_chunk;
        }
    }
}

impl<'a> Drop for RepairPeersManager<'a> {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}