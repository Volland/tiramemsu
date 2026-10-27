//! Row encodings and the scans shared by readers and writers.

use redb::ReadableTable;
use tm_core::term::TermKey;
use tm_core::{Error, Result};
use tm_store::{EmbeddingRow, Pattern, StmtRow, TermRow, TextDoc};

use crate::keys::{at, enc, key, opt, prefix_end, unopt};

/// Maps a redb error to [`Error::Storage`].
pub fn st(e: impl std::fmt::Display) -> Error {
    Error::storage(format!("redb: {e}"))
}

fn pos(r: &StmtRow, i: usize) -> i64 {
    match i {
        0 => r.s,
        1 => r.p,
        _ => r.o,
    }
}

/// The `stmt` value of a row.
pub fn stmt_value(r: &StmtRow) -> [u8; 64] {
    let mut v = [0u8; 64];
    let parts = [
        r.s,
        r.p,
        r.o,
        r.t_add,
        opt(r.t_ret),
        opt(r.v_from),
        opt(r.v_to),
        opt(r.ret_kind),
    ];
    for (i, x) in parts.iter().enumerate() {
        v[i * 8..i * 8 + 8].copy_from_slice(&enc(*x));
    }
    v
}

/// A row from its eid and `stmt` value.
pub fn stmt_row(eid: i64, v: &[u8; 64]) -> StmtRow {
    StmtRow {
        eid,
        s: at(v, 0),
        p: at(v, 1),
        o: at(v, 2),
        t_add: at(v, 3),
        t_ret: unopt(at(v, 4)),
        v_from: unopt(at(v, 5)),
        v_to: unopt(at(v, 6)),
        ret_kind: unopt(at(v, 7)),
    }
}

/// The key of `r` in a live permutation of `order`.
pub fn live_key(order: [usize; 3], r: &StmtRow) -> Vec<u8> {
    key(&[pos(r, order[0]), pos(r, order[1]), pos(r, order[2]), r.eid])
}

/// The value of `r` in every live permutation.
pub fn live_value(r: &StmtRow) -> [u8; 24] {
    let mut v = [0u8; 24];
    v[0..8].copy_from_slice(&enc(r.t_add));
    v[8..16].copy_from_slice(&enc(opt(r.v_from)));
    v[16..24].copy_from_slice(&enc(opt(r.v_to)));
    v
}

/// The key of `r` in a history permutation of `order` (newest addition first).
pub fn hist_key(order: [usize; 3], r: &StmtRow) -> Vec<u8> {
    key(&[
        pos(r, order[0]),
        pos(r, order[1]),
        pos(r, order[2]),
        i64::MAX - r.t_add,
        r.eid,
    ])
}

/// The value of `r` in every history permutation.
pub fn hist_value(r: &StmtRow) -> [u8; 32] {
    let mut v = [0u8; 32];
    v[0..8].copy_from_slice(&enc(opt(r.t_ret)));
    v[8..16].copy_from_slice(&enc(opt(r.ret_kind)));
    v[16..24].copy_from_slice(&enc(opt(r.v_from)));
    v[24..32].copy_from_slice(&enc(opt(r.v_to)));
    v
}

fn place(order: [usize; 3], k: &[u8]) -> [i64; 3] {
    let mut spo = [0i64; 3];
    for (i, o) in order.iter().enumerate() {
        spo[*o] = at(k, i);
    }
    spo
}

/// A live row from its permutation key and value.
pub fn live_row(order: [usize; 3], k: &[u8], v: &[u8; 24]) -> StmtRow {
    let [s, p, o] = place(order, k);
    StmtRow {
        eid: at(k, 3),
        s,
        p,
        o,
        t_add: at(v, 0),
        t_ret: None,
        v_from: unopt(at(v, 1)),
        v_to: unopt(at(v, 2)),
        ret_kind: None,
    }
}

/// A row from its history key and value.
pub fn hist_row(order: [usize; 3], k: &[u8], v: &[u8; 32]) -> StmtRow {
    let [s, p, o] = place(order, k);
    StmtRow {
        eid: at(k, 4),
        s,
        p,
        o,
        t_add: i64::MAX - at(k, 3),
        t_ret: unopt(at(v, 0)),
        v_from: unopt(at(v, 2)),
        v_to: unopt(at(v, 3)),
        ret_kind: unopt(at(v, 1)),
    }
}

/// The key prefix of the bound positions of `pat` in `order`.
pub fn prefix(order: [usize; 3], pat: &Pattern) -> Vec<u8> {
    let bound = [pat.s, pat.p, pat.o];
    let mut parts = Vec::new();
    for o in order {
        match bound[o] {
            Some(x) => parts.push(x),
            None => break,
        }
    }
    key(&parts)
}

/// The rows of a live permutation whose key starts with `pre`, filtered by `pat`.
pub fn scan_live<'t, T>(
    t: &'t T,
    order: [usize; 3],
    pre: Vec<u8>,
    pat: Pattern,
) -> Result<impl Iterator<Item = Result<StmtRow>> + 't>
where
    T: ReadableTable<&'static [u8], [u8; 24]>,
{
    let range = match prefix_end(&pre) {
        Some(end) => t.range::<&[u8]>(pre.as_slice()..end.as_slice()),
        None => t.range::<&[u8]>(pre.as_slice()..),
    }
    .map_err(st)?;
    Ok(range.filter_map(move |e| match e {
        Ok((k, v)) => {
            let r = live_row(order, k.value(), &v.value());
            pat.matches(&r).then_some(Ok(r))
        }
        Err(e) => Some(Err(st(e))),
    }))
}

/// The rows of a history permutation whose key starts with `pre`, filtered by `pat`.
pub fn scan_hist<'t, T>(
    t: &'t T,
    order: [usize; 3],
    pre: Vec<u8>,
    pat: Pattern,
) -> Result<impl Iterator<Item = Result<StmtRow>> + 't>
where
    T: ReadableTable<&'static [u8], [u8; 32]>,
{
    let range = match prefix_end(&pre) {
        Some(end) => t.range::<&[u8]>(pre.as_slice()..end.as_slice()),
        None => t.range::<&[u8]>(pre.as_slice()..),
    }
    .map_err(st)?;
    Ok(range.filter_map(move |e| match e {
        Ok((k, v)) => {
            let r = hist_row(order, k.value(), &v.value());
            pat.matches(&r).then_some(Ok(r))
        }
        Err(e) => Some(Err(st(e))),
    }))
}

/// Every row of the `stmt` table in eid order, optionally live only.
pub fn scan_stmt<'t, T>(
    t: &'t T,
    live_only: bool,
) -> Result<impl Iterator<Item = Result<StmtRow>> + 't>
where
    T: ReadableTable<i64, [u8; 64]>,
{
    let range = t.range::<i64>(..).map_err(st)?;
    Ok(range.filter_map(move |e| match e {
        Ok((k, v)) => {
            let r = stmt_row(k.value(), &v.value());
            (!live_only || r.live()).then_some(Ok(r))
        }
        Err(e) => Some(Err(st(e))),
    }))
}

/// The encoded identity key of a term.
pub fn term_key(k: &TermKey) -> Vec<u8> {
    let lang = k.lang.as_deref().unwrap_or("");
    let mut out = Vec::with_capacity(11 + lang.len() + k.lex.len());
    out.push(k.tag);
    out.extend_from_slice(&enc(opt(k.dt)));
    out.extend_from_slice(&(lang.len() as u16).to_be_bytes());
    out.extend_from_slice(lang.as_bytes());
    out.extend_from_slice(k.lex.as_bytes());
    out
}

/// The encoded value of a term.
pub fn term_value(r: &TermRow) -> Vec<u8> {
    let lang = r.lang.as_deref();
    let mut out = Vec::with_capacity(20 + r.lex.len());
    out.push(r.tag);
    out.extend_from_slice(&enc(opt(r.dt)));
    match r.num {
        Some(n) => {
            out.push(1);
            out.extend_from_slice(&n.to_bits().to_be_bytes());
        }
        None => {
            out.push(0);
            out.extend_from_slice(&[0; 8]);
        }
    }
    match lang {
        Some(l) => {
            out.push(1);
            out.extend_from_slice(&(l.len() as u16).to_be_bytes());
            out.extend_from_slice(l.as_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(r.lex.as_bytes());
    out
}

/// A term from its encoded value.
pub fn term_row(b: &[u8]) -> Result<TermRow> {
    let bad = || Error::storage("redb: damaged term record");
    let tag = *b.first().ok_or_else(bad)?;
    let dt = unopt(at(b.get(1..9).ok_or_else(bad)?, 0));
    let num = match b.get(9) {
        Some(1) => {
            let mut a = [0u8; 8];
            a.copy_from_slice(b.get(10..18).ok_or_else(bad)?);
            Some(f64::from_bits(u64::from_be_bytes(a)))
        }
        Some(_) => None,
        None => return Err(bad()),
    };
    let (lang, rest) = match b.get(18) {
        Some(1) => {
            let n = u16::from_be_bytes([b[19], b[20]]) as usize;
            let l = std::str::from_utf8(b.get(21..21 + n).ok_or_else(bad)?).map_err(|_| bad())?;
            (Some(l.to_string()), &b[21 + n..])
        }
        Some(_) => (None, &b[19..]),
        None => return Err(bad()),
    };
    Ok(TermRow {
        tag,
        lex: std::str::from_utf8(rest).map_err(|_| bad())?.to_string(),
        dt,
        lang,
        num,
    })
}

/// The encoded value of an embedding.
pub fn embedding_value(r: &EmbeddingRow) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + r.bits.len() + r.vec.len());
    out.extend_from_slice(&enc(r.t));
    out.extend_from_slice(&(r.bits.len() as u32).to_be_bytes());
    out.extend_from_slice(&r.bits);
    out.extend_from_slice(&r.vec);
    out
}

/// An embedding from its key parts and encoded value.
pub fn embedding_row(model: i64, eid: i64, b: &[u8]) -> EmbeddingRow {
    let n = u32::from_be_bytes([b[8], b[9], b[10], b[11]]) as usize;
    EmbeddingRow {
        model,
        eid,
        t: at(b, 0),
        bits: b[12..12 + n].to_vec(),
        vec: b[12 + n..].to_vec(),
    }
}

/// The encoded text document.
pub fn text_doc_value(d: &TextDoc) -> Vec<u8> {
    let mut out = d.len.to_be_bytes().to_vec();
    if let Some(l) = &d.lang {
        out.extend_from_slice(l.as_bytes());
    }
    out
}

/// A text document from its encoded value.
pub fn text_doc(b: &[u8]) -> TextDoc {
    TextDoc {
        len: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        lang: (b.len() > 4).then(|| String::from_utf8_lossy(&b[4..]).into_owned()),
    }
}

/// The posting key `token 0x00 value`.
pub fn post_key(token: &str, value: i64) -> Vec<u8> {
    let mut k = Vec::with_capacity(token.len() + 9);
    k.extend_from_slice(token.as_bytes());
    k.push(0);
    k.extend_from_slice(&enc(value));
    k
}

/// Token positions as bytes.
pub fn positions_value(p: &[u32]) -> Vec<u8> {
    p.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// Token positions from bytes.
pub fn positions(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
