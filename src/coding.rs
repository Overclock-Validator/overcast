use serde::{Serialize, Deserialize};
use base64::{Engine as _, engine::general_purpose};
use solana_ledger::shred::{CodingShredHeader, ReedSolomonCache, Shred, ShredType, Shredder};
use ahash::{HashMap, HashMapExt, HashSetExt};
use solana_entry::entry::Entry;
use solana_ledger::shred;
use solana_ledger::shred::ShredType::Data;
pub fn sort_shreds_by_type(shreds: Vec<Shred>) -> (Vec<Shred>, Vec<Shred>) {
    let mut data_shreds: Vec<Shred> = vec![];
    let mut coding_shreds: Vec<Shred> = vec![];

    for s in shreds {
        if s.shred_type() == Data {
            data_shreds.push(s);
        } else {
            coding_shreds.push(s);
        }
    }

    data_shreds.sort_by_key(|x| x.index());
    coding_shreds.sort_by_key(|x| x.index());

    (data_shreds, coding_shreds)
}

pub fn process_shreds_with_recovery(shreds: Vec<Shred>) -> (Vec<Shred>, Vec<Shred>) {
    let rc_cache = ReedSolomonCache::default();

    // Perform recovery on shreds
    let recovery: Vec<Shred> = match shred::recover(shreds.clone(), &rc_cache) {
        Ok(recovered) => recovered.map(|x| x.unwrap()).collect(),
        Err(_) => vec![],
    };

    // Combine original and recovered shreds
    let all_shreds: Vec<Shred> = [shreds, recovery].concat();

    // Sort into data and coding shreds
    sort_shreds_by_type(all_shreds)
}

pub fn get_coding_header_for_fec_set(shreds: &[Shred]) -> Option<CodingShredHeader> {
    for shred in shreds {
        if shred.shred_type() != ShredType::Data {
            match shred {
                Shred::ShredCode(code) => {
                    return Some(*code.coding_header());
                }
                _ => {}
            }
        }
    }
    None
}

pub fn deshred_to_entries(data_shreds: &[Shred]) -> Result<Vec<Entry>, Box<dyn std::error::Error>> {
    let shreds = data_shreds.iter().map(Shred::payload);
    let deshred_payload = Shredder::deshred(shreds)?;
    let deshred_entries: Vec<Entry> = bincode::deserialize(&deshred_payload)?;
    Ok(deshred_entries)
}

pub fn load_shreds_from_json(json_path: &str) -> Result<HashMap<u32, Vec<Shred>>, Box<dyn std::error::Error>> {
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

    let json_str = std::fs::read_to_string(json_path)?;
    let transaction_data: TransactionData = serde_json::from_str(&json_str)?;

    let mut fec_set_map: HashMap<u32, Vec<Shred>> = HashMap::new();

    for s in transaction_data.shreds {
        let decoded = general_purpose::STANDARD.decode(&s.data)?;
        let deser_shred = Shred::new_from_serialized_shred(decoded)?;

        fec_set_map.entry(deser_shred.fec_set_index())
            .or_insert_with(Vec::new)
            .push(deser_shred);
    }

    Ok(fec_set_map)
}
#[cfg(test)]
pub mod test {

    use super::*;

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
        // Load shreds from JSON file
        let fec_set_map = load_shreds_from_json("src/test_data/slot_shreds.json")
            .expect("Failed to load and parse shreds");

        let mut fec_indices = fec_set_map.keys().collect::<Vec<&u32>>();
        fec_indices.sort();
        println!("{:#?}", fec_indices);
        // Get shreds for FEC set 0
        let shreds = fec_set_map.get(& 890u32).unwrap().clone();

        if let Some(coding_header) = get_coding_header_for_fec_set(&shreds) {
            println!("Coding header: {:?}", coding_header);
            println!("Number of data shreds: {}", coding_header.num_data_shreds);
            println!("Number of coding shreds: {}", coding_header.num_coding_shreds);
        }

        for s in &shreds {
            println!("{:?}, {}, {:#?}",s.shred_type(), s.index(), s.last_in_slot());
        }

        // Process shreds with recovery and sort by type
        let (data_shreds, coding_shreds) = process_shreds_with_recovery(shreds);

        // Get coding header information if available

        // match deshred_to_entries(&data_shreds) {
        //     Ok(entries) => {
        //         println!("Deserialized entries: {:#?}", entries);
        //     },
        //     Err(err) => {
        //         println!("Failed to deshred entries: {}", err);
        //     }
        // }
    }

}
