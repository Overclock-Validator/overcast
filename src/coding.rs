#[cfg(test)]
pub mod test {
    use serde::{Serialize, Deserialize};
    use base64::{Engine as _, engine::general_purpose};
    use solana_ledger::shred::{ReedSolomonCache, Shred, Shredder};
    use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
    use solana_entry::entry::Entry;
    use solana_ledger::shred;
    use solana_ledger::shred::ShredType::Data;

    #[derive(Debug, Deserialize, Serialize)]
    struct TransactionShred {
        slot: u64,
        data: String,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct TransactionData {
        slot: u64,
        shreds: Vec<TransactionShred>,
    }


    #[test]
    fn assemble() {
        let json_str = include_str!("test_data/slot_shreds.json");
        let transaction_data: TransactionData = serde_json::from_str(json_str)
            .expect("Failed to parse JSON");

        let num_shreds = transaction_data.shreds.len();

        let mut fec_set_map: HashMap<u32, Vec<Shred>> = HashMap::new();

        for s in transaction_data.shreds {
            let decoded = general_purpose::STANDARD.decode(&s.data)
                .expect("Failed to decode base64 string");
            let deser_shred = Shred::new_from_serialized_shred(decoded).unwrap();

            fec_set_map.entry(deser_shred.fec_set_index())
                .or_insert_with(Vec::new)
                .push(deser_shred);

        }

        let rc_cache = ReedSolomonCache::default();
        let shreds = fec_set_map.get(&0u32).unwrap().clone();
        let recovery : Vec<Shred> = shred::recover(shreds.clone(), &rc_cache).unwrap().map(|x| x.unwrap()).collect();

        let mut data_shreds: Vec<Shred> = vec![];
        for s in shreds {
            if s.shred_type() != Data {
                continue
            }
            let idx = s.index() as usize;
            data_shreds.push(s.clone())
        }
        for r in recovery {
            if r.shred_type() != Data {
                continue
            }
            let idx = r.index() as usize;
            data_shreds.push(r.clone())
        }
        data_shreds.sort_by_key(|x|x.index());

        let deshred_payload = {
            let shreds = data_shreds.iter().map(Shred::payload);
            Shredder::deshred(shreds).unwrap()
        };
        let deshred_entries: Vec<Entry> = bincode::deserialize(&deshred_payload).unwrap();
        println!("{:#?}", deshred_entries);
    }

}
