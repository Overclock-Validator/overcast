#[cfg(test)]
pub mod test {
    use serde::{Serialize, Deserialize};
    use serde_json::Value;
    use solana_ledger::blockstore::Blockstore;
    use base64::{Engine as _, engine::general_purpose};
    use solana_ledger::shred::{ReedSolomonCache, Shred, Shredder};
    use std::collections::{BTreeMap, BTreeSet};
    use std::ops::Range;
    use ahash::{HashMap, HashMapExt};

    #[derive(Debug, Deserialize, Serialize)]
    struct TransactionShred {
        slot: u64,
        shred_index: u32,
        data: String,
        length: u64,
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
        let mut completed_shred_indices = BTreeSet::new();
        let mut consumed = 0u32;
        let mut received = 0u32;
        let mut last_index: Option<u32> = None;
        let num_shreds = transaction_data.shreds.len();
        let mut all_indices = vec![false; num_shreds];

        let mut fec_set_map: HashMap<u32, Vec<Shred>> = HashMap::new();

        for s in transaction_data.shreds {
            let decoded = general_purpose::STANDARD.decode(&s.data)
                .expect("Failed to decode base64 string");
            let deser_shred = Shred::new_from_serialized_shred(decoded).unwrap();
            println!("{} {} {:?}", deser_shred.index(), deser_shred.fec_set_index(), deser_shred.shred_type());

            fec_set_map.entry(deser_shred.fec_set_index())
                .or_insert_with(Vec::new)
                .push(deser_shred);

                // panic!("go");
                completed_shred_indices.insert(s.shred_index);
                if s.shred_index < num_shreds as u32 {
                    all_indices[s.shred_index as usize] = true;
                }
                if s.shred_index > received {
                    received = s.shred_index;
                }
                // if deser_shred.last_in_slot() {
                //     last_index = Some(s.shred_index);
                // }
            }
            consumed = (num_shreds-1) as u32;
            for (c, &present) in all_indices.iter().enumerate() {
                if !present {
                    consumed = c as u32;
                    break;
                }
            }

            let completed_ranges: Vec<Range<u32>> = completed_shred_indices
                .range(0..consumed)
                .scan(0, |start, &index| {
                    let out = *start..index + 1;
                    *start = index + 1;
                    Some(out)
                })
                .collect();

            // Shredder::deshred(range_shreds);

            // consumed: first hole index starting from 0
            // received: highest shred index + 1
            // last_index: LAST_SHRED_IN_SLOT flag set

            // let completed_ranges = Self::get_completed_data_ranges(
            //     start_index as u32,
            //     &slot_meta.completed_data_indexes,
            //     slot_meta.consumed as u32,
            // );

            // let Some((all_ranges_start_index, _)) = completed_ranges.first().copied() else {
            //     return Ok(vec![]);
            // };
            // let Some((_, all_ranges_end_index)) = completed_ranges.last().copied() else {
            //     return Ok(vec![]);
            // };

            // let keys =
            //     (all_ranges_start_index..=all_ranges_end_index).map(|index| (slot, u64::from(index)));

            // let data_shreds: Result<Vec<Option<Vec<u8>>>> = self
            //     .data_shred_cf
            //     .multi_get_bytes(keys)
            //     .into_iter()
            //     .collect();

            // let range_start_index = (start_index - all_ranges_start_index) as usize;
            // let range_end_index = (end_index - all_ranges_start_index) as usize;
            // let range_shreds = &data_shreds[range_start_index..=range_end_index];
            //
            // let last_shred = range_shreds.last().unwrap();
            // assert!(last_shred.data_complete() || last_shred.last_in_slot());
            // trace!("{:?} data shreds in last FEC set", data_shreds.len());
            //
            // Shredder::deshred(range_shreds)
        let rc_cache = ReedSolomonCache::default();
        let shreds = fec_set_map.get(&0u32).unwrap().clone();
        println!("{:?}",shreds.len());
        let rec = Shredder::try_recovery(shreds, &rc_cache).unwrap();
    }

}
