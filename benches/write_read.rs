use std::fs;
use std::ops::Bound::Unbounded;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options, WriteOptions};

fn bench_sequential_write(c: &mut Criterion) {
    c.bench_function("sequential_write", |b| {
        b.iter_batched(
            || fresh_db("sequential_write"),
            |db| {
                for index in 0..32 {
                    let key = format!("seq-{index:04}");
                    db.put(key.as_bytes(), b"value").expect("put");
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_random_write(c: &mut Criterion) {
    c.bench_function("random_write", |b| {
        b.iter_batched(
            || fresh_db("random_write"),
            |db| {
                for index in 0..32 {
                    let shuffled = (index * 17) % 37;
                    let key = format!("rand-{shuffled:04}");
                    db.put(key.as_bytes(), b"value").expect("put");
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_point_lookup_hit(c: &mut Criterion) {
    c.bench_function("point_lookup_hit", |b| {
        let db = fresh_db("point_lookup_hit");
        for index in 0..64 {
            let key = format!("hit-{index:04}");
            db.put(key.as_bytes(), b"value").expect("put");
        }
        b.iter(|| {
            assert_eq!(db.get(b"hit-0032").expect("get"), Some(b"value".to_vec()));
        });
    });
}

fn bench_point_lookup_miss(c: &mut Criterion) {
    c.bench_function("point_lookup_miss", |b| {
        let db = fresh_db("point_lookup_miss");
        for index in 0..64 {
            let key = format!("miss-base-{index:04}");
            db.put(key.as_bytes(), b"value").expect("put");
        }
        b.iter(|| {
            assert_eq!(db.get(b"not-present").expect("get"), None);
        });
    });
}

fn bench_range_scan(c: &mut Criterion) {
    c.bench_function("range_scan", |b| {
        let db = fresh_db("range_scan");
        for index in 0..96 {
            let key = format!("scan-{index:04}");
            db.put(key.as_bytes(), b"value").expect("put");
        }
        b.iter(|| {
            assert_eq!(db.scan(Unbounded, Unbounded).expect("scan").len(), 96);
        });
    });
}

fn bench_wal_sync_write(c: &mut Criterion) {
    c.bench_function("wal_sync_write_latency", |b| {
        b.iter_batched(
            || fresh_db("wal_sync_write_latency"),
            |db| {
                db.write(
                    {
                        let mut batch = tylsmdb::WriteBatch::new();
                        batch.put(b"sync-key".to_vec(), b"value".to_vec());
                        batch
                    },
                    WriteOptions {
                        sync: true,
                        disable_wal: false,
                    },
                )
                .expect("sync write");
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_flush_and_compaction(c: &mut Criterion) {
    c.bench_function("flush_and_compaction_throughput", |b| {
        b.iter_batched(
            || fresh_db("flush_and_compaction_throughput"),
            |db| {
                for round in 0..2 {
                    for index in 0..24 {
                        let key = format!("compact-{index:04}");
                        let value = format!("value-{round}-{index}");
                        db.put(key.as_bytes(), value.as_bytes()).expect("put");
                    }
                    db.flush().expect("flush");
                }
                db.compact_range(Unbounded, Unbounded).expect("compact");
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn fresh_db(name: &str) -> DB {
    static NEXT_BENCH_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_BENCH_ID.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from("target/tylsmdb-benches").join(format!("{name}-{id}"));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create bench dir");
    DB::open(
        &path,
        Options {
            memtable_kind: MemTableKind::SkipList,
            table_compression: CompressionType::Zstd,
            memtable_size: 1024,
            block_size: 1024,
            target_file_size_base: 4096,
            max_subcompactions: 2,
            ..Options::default()
        },
    )
    .expect("open bench db")
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_secs(1));
    targets =
        bench_sequential_write,
        bench_random_write,
        bench_point_lookup_hit,
        bench_point_lookup_miss,
        bench_range_scan,
        bench_wal_sync_write,
        bench_flush_and_compaction
}
criterion_main!(benches);
