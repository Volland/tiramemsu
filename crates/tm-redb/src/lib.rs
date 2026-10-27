#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod keys;
mod rows;
mod store;
mod tables;

use std::io::Read;
use std::path::Path;

pub use store::{Durability, RedbOptions, RedbRead, RedbStore, RedbWrite, LAYOUT, MAGIC};

/// True when the file at `path` starts with the redb magic number.
pub fn is_redb_file(path: &Path) -> bool {
    let mut head = [0u8; 9];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|_| head == MAGIC)
}
