use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::{self, Read};
use std::ops::Bound::Unbounded;
use std::path::{Path, PathBuf};

use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options};

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    run_case(&input)
}

fn run_case(input: &[u8]) -> Result<(), Box<dyn Error>> {
    let path = fuzz_dir(input);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path)?;

    let mut db = DB::open(&path, fuzz_options())?;
    let mut oracle = BTreeMap::<Vec<u8>, Vec<u8>>::new();

    for chunk in input.chunks(3) {
        let op = chunk.first().copied().unwrap_or_default() % 7;
        let key_id = chunk.get(1).copied().unwrap_or_default() % 32;
        let value_id = chunk.get(2).copied().unwrap_or_default();
        let key = format!("fk-{key_id:02}").into_bytes();
        let value = format!("fv-{value_id:03}").into_bytes();

        match op {
            0 | 1 => {
                db.put(&key, &value)?;
                oracle.insert(key, value);
            }
            2 => {
                db.delete(&key)?;
                oracle.remove(&key);
            }
            3 => {
                assert_eq!(db.get(&key)?, oracle.get(&key).cloned());
            }
            4 => assert_scan_matches(&db, &oracle)?,
            5 => db.flush()?,
            _ => {
                db.compact_range(Unbounded, Unbounded)?;
                drop(db);
                db = DB::open(&path, fuzz_options())?;
            }
        }
        assert_scan_matches(&db, &oracle)?;
    }

    let _ = fs::remove_dir_all(&path);
    Ok(())
}

fn assert_scan_matches(db: &DB, oracle: &BTreeMap<Vec<u8>, Vec<u8>>) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        db.scan(Unbounded, Unbounded)?,
        oracle
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>()
    );
    Ok(())
}

fn fuzz_options() -> Options {
    Options {
        memtable_kind: MemTableKind::SkipList,
        table_compression: CompressionType::Zstd,
        memtable_size: 256,
        block_size: 256,
        target_file_size_base: 512,
        max_subcompactions: 3,
        ..Options::default()
    }
}

fn fuzz_dir(input: &[u8]) -> PathBuf {
    let hash = input.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        hash.wrapping_mul(0x1000_0000_01b3) ^ u64::from(*byte)
    });
    Path::new("target")
        .join("tylsmdb-fuzz")
        .join(format!("fuzz-model-{}-{hash:016x}", std::process::id()))
}
