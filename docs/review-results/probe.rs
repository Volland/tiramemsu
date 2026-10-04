use std::{sync::Arc, time::Instant};
use tiramemsu::*;
fn v(s: &str) -> Value { Value::iri(format!("urn:tiramemsu:v:{s}")) }
fn main() -> Result<()> {
    for readers in [0,1,4,8] {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(dir.path().join("probe.db"), OpenOptions { readers, ..Default::default() })?);
        db.transact(TxOptions::default(), |tx| { for i in 0..100 { tx.assert(v(&format!("n{i}")),v("label"),Value::str(format!("label{i}")),Valid::ALWAYS)?; } Ok(()) })?;
        let start = Instant::now();
        let workers: Vec<_> = (0..8).map(|_| { let db=db.clone(); std::thread::spawn(move || { for _ in 0..500 { let r=db.now().sparql("SELECT ?l WHERE { v:n42 v:label ?l }").unwrap(); assert_eq!(r.solutions().unwrap().rows.len(),1); } }) }).collect();
        for worker in workers { worker.join().unwrap(); }
        println!("readers={readers} workers=8 queries=4000 elapsed_ms={:.2}",start.elapsed().as_secs_f64()*1000.0);
    }
    let history_dir=tempfile::tempdir().unwrap();
    let history=Db::open(history_dir.path().join("history.db"),Default::default())?;
    history.transact(Default::default(),|tx| tx.assert(v("score"),Value::iri(vocab::SYS_CARDINALITY),Value::iri(vocab::SYS_ONE),Valid::ALWAYS).map(|_| ()))?;
    for u in 0..1000 { history.transact(Default::default(),|tx| { for k in 0..20 {tx.assert(v(&format!("k{k}")),v("score"),Value::Int(u),Valid::ALWAYS)?;} Ok(()) })?; }
    let hs=history.now().encode(&v("k7"))?.unwrap().raw();
    let hp=history.now().encode(&v("score"))?.unwrap().raw();
    println!("churn_now_plan={:?}",history.read_sql(&format!("EXPLAIN QUERY PLAN SELECT a.eid,a.s,a.p,a.o,a.t_add,a.t_ret,a.v_from,a.v_to,a.ret_kind FROM triple a WHERE a.s={hs} AND a.p={hp} AND a.t_ret IS NULL ORDER BY a.eid"))?);
    for states in [10,10000] {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("path.db"),OpenOptions { path_max_states: states,..Default::default() })?;
        db.transact(TxOptions::default(), |tx| { for i in 0..100 { tx.assert(v(&format!("n{i}")),v("next"),v(&format!("n{}",i+1)),Valid::from(1000))?; } Ok(()) })?;
        let id=db.now().encode(&v("n0"))?.unwrap();
        for hops in [3,15,100] {
            let start=Instant::now();
            let result=db.now().path(id,"next+",PathMode::Trail,hops);
            println!("states={states} hops={hops} elapsed_us={} result={:?}",start.elapsed().as_micros(),result.as_ref().map(|r| r.len()));
        }
        for time in [999,1000] { println!("valid_at={time} result={:?}",db.now().valid_at(time).path(id,"next+",PathMode::Reachability,u32::MAX).as_ref().map(|r| r.len())); }
    }
    for cache in [0,16,16384] {
        let cache_dir=tempfile::tempdir().unwrap();
        let cache_db=Db::open(cache_dir.path().join("cache.db"),OpenOptions {term_cache_capacity:cache,..Default::default()})?;
        cache_db.transact(Default::default(),|tx| tx.assert(v("a"),v("p"),Value::str("a long string requiring dictionary decoding"),Valid::ALWAYS).map(|_| ()))?;
        let start=Instant::now();
        for _ in 0..1000 { let r=cache_db.now().sparql("SELECT ?o WHERE { v:a v:p ?o }")?; assert_eq!(r.solutions().unwrap().rows.len(),1); }
        println!("term_cache_capacity={cache} queries=1000 elapsed_ms={:.2}",start.elapsed().as_secs_f64()*1000.0);
    }
    let core_dir=tempfile::tempdir().unwrap();
    let core=Db::open(core_dir.path().join("core.db"),OpenOptions {query_engine:false,..Default::default()})?;
    core.transact(Default::default(),|tx| tx.assert(v("a"),v("p"),Value::Int(1),Valid::ALWAYS).map(|_| ()))?;
    println!("query_engine=false triple_rows={} sparql={:?}",core.now().triples(None,None,None)?.len(),core.now().sparql("SELECT ?o WHERE { v:a v:p ?o }"));
    let dir = tempfile::tempdir().unwrap();
    let db=Db::open(dir.path().join("panic.db"),Default::default())?;
    let panic=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { let _=db.transact(Default::default(),|tx| { tx.assert(v("a"),v("p"),Value::Int(1),Valid::ALWAYS)?; panic!("intentional transaction callback panic"); }); }));
    println!("panic_caught={} subsequent_write={:?}",panic.is_err(),db.transact(Default::default(),|tx| tx.assert(v("b"),v("p"),Value::Int(2),Valid::ALWAYS).map(|_| ())));
    println!("post_panic_committed_rows={}",db.now().triples(None,None,None)?.len());
    let reader_dir=tempfile::tempdir().unwrap();
    let reader_db=Arc::new(Db::open(reader_dir.path().join("read-panic.db"),OpenOptions {readers:1,..Default::default()})?);
    reader_db.set_query_hook(Some(Arc::new(|| panic!("intentional reader callback panic"))));
    let panic=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {let _=reader_db.now().sparql("SELECT ?s WHERE { ?s ?p ?o }");}));
    reader_db.set_query_hook(None);
    println!("reader_panic_caught={} reader_count={} (follow-up read checked with a timeout)",panic.is_err(),reader_db.reader_count());
    let (sender,receiver)=std::sync::mpsc::channel();
    let stuck=reader_db.clone();
    std::thread::spawn(move || { let r=stuck.now().triples(None,None,None); let _=sender.send(r); });
    println!("reader_after_panic_completed_within_250ms={}",receiver.recv_timeout(std::time::Duration::from_millis(250)).is_ok());
    for optimize_every in [1000,1000000] {
        let bulk_dir=tempfile::tempdir().unwrap();
        let bulk=Db::open(bulk_dir.path().join("bulk.db"),OpenOptions {optimize_every,..Default::default()})?;
        let start=Instant::now();
        for chunk in 0..20 { bulk.transact(Default::default(),|tx| { for i in chunk*1000..(chunk+1)*1000 { tx.assert(v(&format!("n{i}")),v("p"),Value::Int(i),Valid::ALWAYS)?; } Ok(()) })?; }
        println!("bulk rows=20000 chunk=1000 optimize_every={optimize_every} analyze_runs={} elapsed_ms={:.2}",bulk.optimize_runs()?,start.elapsed().as_secs_f64()*1000.0);
    }
    Ok(())
}
