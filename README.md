# Overcast

Overcast will serve as a **lightweight Solana “edge” node** that caches and republishes *ephemeral* Turbine and repair data so other software can fetch recent blocks without the expense of running a full RPC validator or relying expensive subscription services from more centralized RPC providers to frequently request full blocks.  

By stripping out the Accounts DB, vote engine, and RPC layer, an Overcast instance fits comfortably on the same machine as heavier tools—or on a tiny VPS—while still streaming fresh blocks to local indexers, monitoring agents, or anything else that needs them.

This project is **under active development**. Code and docs will evolve quickly; expect rough edges until we tag the first release.

### Milestone 1 (In progress): Core Turbine / Repair Pipeline
* Ingest incoming shreds into a rolling cache (default retention ≈ 1 h, but fully configurable).  
* Detect gaps rapidly and issue repair requests.  
* Validate repair responses and re‑assemble blocks.  
* Serve valid shreds to requesting peers.

### Milestone 2 (Planned): Mithril Integration
* Expose a simple local API so Mithril can stream live blocks directly from Overcast instead of a centralized RPC.  
* Support co‑location: run Overcast on the same host as Mithril with minimal extra resource overhead.  
* Allow optional mesh discovery so a cluster of Overcast nodes can backstop each other for block availability.

### Milestone 3 (Future): Mesh & Configurability
* Peer discovery to form regional cache meshes.  
* Tunable retention policies and hard resource caps.  


