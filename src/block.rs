use chrono::prelude::*;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DIFFICULTY_PREFIX: &str = "00";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: u64,
    pub hash: String,
    pub previous_hash: String,
    pub timestamp: i64,
    pub data: String,
    pub nonce: u64,
}

impl Block {
    pub fn mine(id: u64, previous_hash: String, data: String) -> Self {
        let now = Utc::now();
        let (nonce, hash) = mine_block(id, now.timestamp(), &previous_hash, &data);
        Self {
            id,
            hash,
            timestamp: now.timestamp(),
            previous_hash,
            data,
            nonce,
        }
    }
}

pub fn new_block_valid(new_block: &Block, previous_block: &Block) -> bool {
    if new_block.id != previous_block.id + 1 {
        warn!(
            "block with id: {} is not the next block after the latest: {}",
            new_block.id, previous_block.id
        );
        return false;
    }
    if new_block.previous_hash != previous_block.hash {
        warn!("block with id: {} has wrong previous hash", new_block.id);
        return false;
    }
    if !has_correct_form(&hex::decode(&new_block.hash).expect("can decode from hex")) {
        warn!(
            "block with id: {} has hash with incorrect form",
            new_block.id
        );
        return false;
    }
    if !hash_consistent_with_other_data(new_block) {
        warn!("block with id: {} has inconsistent hash", new_block.id);
        return false;
    }
    true
}

fn hash_consistent_with_other_data(block: &Block) -> bool {
    hex::encode(calculate_hash(
        block.id,
        block.timestamp,
        &block.previous_hash,
        &block.data,
        block.nonce,
    )) == block.hash
}

fn has_correct_form(hash: &[u8]) -> bool {
    hash_to_binary_representation(hash).starts_with(DIFFICULTY_PREFIX)
}

fn calculate_hash(id: u64, timestamp: i64, previous_hash: &str, data: &str, nonce: u64) -> Vec<u8> {
    let data = serde_json::json!({
        "id": id,
        "previous_hash": previous_hash,
        "data": data,
        "timestamp": timestamp,
        "nonce": nonce
    });
    let mut hasher = Sha256::new();
    hasher.update(data.to_string().as_bytes());
    hasher.finalize().as_slice().to_owned()
}

fn mine_block(id: u64, timestamp: i64, previous_hash: &str, data: &str) -> (u64, String) {
    info!("mining block...");
    let mut nonce = 0;

    loop {
        if nonce % 100000 == 0 {
            info!("nonce: {}", nonce);
        }
        let hash = calculate_hash(id, timestamp, previous_hash, data, nonce);
        if has_correct_form(&hash) {
            let binary_hash = hash_to_binary_representation(&hash);
            info!(
                "mined! nonce: {}, hash: {}, binary hash: {}",
                nonce,
                hex::encode(&hash),
                binary_hash
            );
            return (nonce, hex::encode(hash));
        }
        nonce += 1;
    }
}

fn hash_to_binary_representation(hash: &[u8]) -> String {
    hash.iter().map(|c| format!("{:b}", c)).collect()
}
