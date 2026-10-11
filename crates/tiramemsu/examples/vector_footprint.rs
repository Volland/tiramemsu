//! Measures the on-disk cost of embeddings (`lat.md/storage.md#Measured
//! Footprint`): the same 10 000 statements with no vectors, with 384- and
//! 768-dimension f32 vectors, and with 768-dimension f16 vectors, reporting bytes
//! per embedding for the table and both indexes.
//!
//! Run: `cargo run --release -p tiramemsu --example vector_footprint`.

use tiramemsu::*;

const N: usize = 10_000;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

/// Bytes of the file after a checkpoint and VACUUM, and of the embedding table and
/// its indexes (from `dbstat` when the build has it).
fn measure(dims: Option<(u32, VectorEncoding)>) -> (u64, Vec<(String, u64)>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.db");
    {
        let db = Db::open(&path, OpenOptions::default()).unwrap();
        let model = v("m");
        let mut seed = 1u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((seed >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        };
        if let Some((d, enc)) = dims {
            db.transact(TxOptions::default(), |tx| {
                tx.define_embedding_model(
                    model.clone(),
                    &EmbeddingModel {
                        dimensions: d,
                        metric: Metric::Cosine,
                        encoding: enc,
                    },
                )
                .map(|_| ())
            })
            .unwrap();
        }
        for chunk in (0..N).collect::<Vec<_>>().chunks(2_000) {
            db.transact(TxOptions::default(), |tx| {
                for &i in chunk {
                    let e = tx.create(v("doc"), v("chunk"), Value::Int(i as i64), Valid::ALWAYS)?;
                    if let Some((d, _)) = dims {
                        let x: Vec<f32> = (0..d).map(|_| next()).collect();
                        tx.set_embedding(e, model.clone(), &x)?;
                    }
                }
                Ok(())
            })
            .unwrap();
        }
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
        .unwrap();
    let parts = ["embedding", "embedding_model_eid", "embedding_bits"]
        .iter()
        .filter_map(|name| {
            conn.query_row(
                "SELECT sum(pgsize) FROM dbstat WHERE name = ?1",
                [name],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .map(|b| (name.to_string(), b as u64))
        })
        .collect();
    drop(conn);
    (std::fs::metadata(&path).unwrap().len(), parts)
}

fn main() {
    let (base, _) = measure(None);
    println!("{N} statements without vectors: {base} bytes");
    for (d, enc) in [
        (384, VectorEncoding::F32),
        (768, VectorEncoding::F32),
        (768, VectorEncoding::F16),
    ] {
        let (size, parts) = measure(Some((d, enc)));
        println!(
            "{d} {}: file +{} bytes per embedding (raw vector {} bytes)",
            enc.name(),
            (size - base) / N as u64,
            d as usize * if enc == VectorEncoding::F32 { 4 } else { 2 },
        );
        for (name, bytes) in parts {
            println!("    {name}: {} bytes per embedding", bytes / N as u64);
        }
    }
}
