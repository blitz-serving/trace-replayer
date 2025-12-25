use std::{
    cell::UnsafeCell,
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{token_sampler::TokenSampler, SpinRwLock};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tracing::{instrument, Level};

/// jsonl of Bailian
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BailianDataItem {
    pub chat_id: i64,
    pub parent_chat_id: i64,
    pub timestamp: f64,
    pub input_length: u64,
    pub output_length: u64,
    pub r#type: String,
    pub turn: u64,
    pub hash_ids: Vec<u64>,
}

/// jsonl of Mooncake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MooncakeDataItem {
    pub timestamp: f32,
    pub input_length: u64,
    pub output_length: u64,
    pub hash_ids: Vec<u64>,
}

fn from_timestamp<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    // support "2023-11-16 18:15:46.6805900" format
    NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f").map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AzureDataItem {
    #[serde(rename = "TIMESTAMP", deserialize_with = "from_timestamp")]
    pub naive_timestamp: NaiveDateTime,
    #[serde(rename = "ContextTokens")]
    pub context_tokens: u64,
    #[serde(rename = "GeneratedTokens")]
    pub generated_tokens: u64,
    #[serde(skip)]
    pub timestamp: u64,
}

pub struct DataIter {
    size: usize,
    index: AtomicUsize,
}

impl Iterator for DataIter {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.index.fetch_add(1, Ordering::AcqRel);
        if i >= self.size {
            // fuse the iterator
            self.index.store(i, Ordering::Release);
            return None;
        }
        Some(i)
    }
}

unsafe impl Send for DataIter {}
unsafe impl Sync for DataIter {}

pub trait LLMTrace: Send + Sync {
    fn load(&mut self, path: &str);
    fn timestamp(&self, index: usize) -> u64;
    fn inflate(&self, index: usize, ts: &TokenSampler) -> (String, u64, u64);
    fn iter(&self) -> DataIter;
    fn rps(&self) -> f64;
}

//
// ============== BailianDataset ==============
//
pub struct BailianDataset {
    items: Vec<BailianDataItem>,
    user_prompts: UnsafeCell<HashMap<u64, String>>,
    rwlock: SpinRwLock,
}

impl BailianDataset {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            user_prompts: UnsafeCell::new(HashMap::new()),
            rwlock: SpinRwLock::new(),
        }
    }
}

unsafe impl Send for BailianDataset {}
unsafe impl Sync for BailianDataset {}

impl LLMTrace for BailianDataset {
    fn load(&mut self, path: &str) {
        let file = File::open(path).unwrap();

        for line in BufReader::new(file).lines() {
            let item: BailianDataItem = serde_json::from_str(line.unwrap().as_str()).unwrap();
            self.items.push(item);
        }
    }

    fn iter(&self) -> DataIter {
        DataIter { size: self.items.len(), index: AtomicUsize::new(0) }
    }

    fn rps(&self) -> f64 {
        self.items.len() as f64
            / (self.items.last().unwrap().timestamp - self.items.first().unwrap().timestamp)
    }

    fn timestamp(&self, index: usize) -> u64 {
        (self.items[index].timestamp * 1000.) as u64
    }

    #[instrument(skip_all, target = "inflate", fields(chat_id = index), level = Level::INFO)]
    fn inflate(&self, index: usize, ts: &TokenSampler) -> (String, u64, u64) {
        // NOTE: the last block hash may be hashed onto a partially filled block
        const BLOCK_SIZE: usize = 16;
        unsafe {
            let data_item = self.items.get(index).unwrap();
            let last_block_len =
                (*data_item).input_length as usize - ((*data_item).hash_ids.len() - 1) * BLOCK_SIZE;
            debug_assert!(last_block_len <= BLOCK_SIZE);

            let x = if last_block_len == BLOCK_SIZE { 0 } else { 1 };
            let mut prompt =
                String::with_capacity(usize::next_power_of_two((*data_item).input_length as usize));
            for &hash_id in (*data_item).hash_ids.iter().take((*data_item).hash_ids.len() - x) {
                // loop invariant: rwlock is free
                self.rwlock.read_lock();
                if let Some(s) = (&*self.user_prompts.get()).get(&hash_id) {
                    prompt.push_str(&s);
                    self.rwlock.read_unlock();
                } else {
                    self.rwlock.read_unlock();
                    let s = ts.gen_string(BLOCK_SIZE);
                    self.rwlock.write_lock();
                    if let Some(s0) = (*self.user_prompts.get()).get(&hash_id) {
                        prompt.push_str(&s0);
                    } else {
                        prompt.push_str(&s);
                        (&mut *self.user_prompts.get()).insert(hash_id, s);
                    }
                    self.rwlock.write_unlock();
                }
            }

            if x == 1 {
                let last_block_prompt = ts.gen_string(last_block_len);
                prompt.push_str(&last_block_prompt);
                self.rwlock.write_lock();
                (&mut *self.user_prompts.get())
                    .insert(*(*data_item).hash_ids.last().unwrap(), last_block_prompt);
                self.rwlock.write_unlock();
            }

            (prompt, (*data_item).input_length, (*data_item).output_length)
        }
    }
}

//
// ============== MooncakeDataset ==============
//
pub struct MooncakeDataset {
    items: Vec<MooncakeDataItem>,
    user_prompts: UnsafeCell<HashMap<u64, String>>,
    rwlock: SpinRwLock,
}

impl MooncakeDataset {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            user_prompts: UnsafeCell::new(HashMap::new()),
            rwlock: SpinRwLock::new(),
        }
    }
}

unsafe impl Send for MooncakeDataset {}
unsafe impl Sync for MooncakeDataset {}

impl LLMTrace for MooncakeDataset {
    fn load(&mut self, path: &str) {
        let file = File::open(path).unwrap();
        for line in BufReader::new(file).lines() {
            let item: MooncakeDataItem = serde_json::from_str(line.unwrap().as_str()).unwrap();
            self.items.push(item);
        }
    }

    fn iter(&self) -> DataIter {
        DataIter { size: self.items.len(), index: AtomicUsize::new(0) }
    }

    fn rps(&self) -> f64 {
        self.items.len() as f64
            / (self.items.last().unwrap().timestamp as f64
                - self.items.first().unwrap().timestamp as f64)
    }

    fn timestamp(&self, index: usize) -> u64 {
        (self.items[index].timestamp * 1000.) as u64
    }

    fn inflate(&self, index: usize, ts: &TokenSampler) -> (String, u64, u64) {
        // NOTE: the last block hash may be hashed onto a partially filled block
        const BLOCK_SIZE: usize = 512;
        unsafe {
            let data_item = self.items.get(index).unwrap();
            let last_block_len =
                (*data_item).input_length as usize - ((*data_item).hash_ids.len() - 1) * BLOCK_SIZE;
            debug_assert!(last_block_len <= BLOCK_SIZE);

            let x = if last_block_len == BLOCK_SIZE { 0 } else { 1 };
            let mut prompt =
                String::with_capacity(usize::next_power_of_two((*data_item).input_length as usize));
            for &hash_id in (*data_item).hash_ids.iter().take((*data_item).hash_ids.len() - x) {
                // loop invariant: rwlock is free
                self.rwlock.read_lock();
                if let Some(s) = (&*self.user_prompts.get()).get(&hash_id) {
                    prompt.push_str(&s);
                    self.rwlock.read_unlock();
                } else {
                    self.rwlock.read_unlock();
                    let s = ts.gen_string(BLOCK_SIZE);
                    self.rwlock.write_lock();
                    if let Some(s0) = (*self.user_prompts.get()).get(&hash_id) {
                        prompt.push_str(&s0);
                    } else {
                        prompt.push_str(&s);
                        (&mut *self.user_prompts.get()).insert(hash_id, s);
                    }
                    self.rwlock.write_unlock();
                }
            }
            // postcond: rwlock is free

            if x == 1 {
                let last_block_prompt = ts.gen_string(last_block_len);
                prompt.push_str(&last_block_prompt);
                self.rwlock.write_lock();
                (&mut *self.user_prompts.get())
                    .insert(*(*data_item).hash_ids.last().unwrap(), last_block_prompt);
                self.rwlock.write_unlock();
            }

            (prompt, (*data_item).input_length, (*data_item).output_length)
        }
    }
}
