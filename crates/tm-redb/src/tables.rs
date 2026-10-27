//! The redb tables of layout 1 and the permutations over them.

use redb::TableDefinition;

/// Engine marker and layout version (`engine = "tiramemsu"`, `layout = "1"`).
pub const INFO: TableDefinition<&str, &str> = TableDefinition::new("info");
/// The engine counters and format keys of the `meta` table.
pub const META: TableDefinition<&str, i64> = TableDefinition::new("meta");
/// Term id → encoded term.
pub const TERM: TableDefinition<i64, &[u8]> = TableDefinition::new("term");
/// Encoded term key → term id.
pub const TERM_KEY: TableDefinition<&[u8], i64> = TableDefinition::new("term_key");
/// Transaction number → instant.
pub const TX: TableDefinition<i64, i64> = TableDefinition::new("tx");
/// Instant → transaction number.
pub const TX_INSTANT: TableDefinition<i64, i64> = TableDefinition::new("tx_instant");
/// Eid → the whole statement row (eight encoded integers).
pub const STMT: TableDefinition<i64, [u8; 64]> = TableDefinition::new("stmt");
/// `(t_add, eid)` of every statement.
pub const LOG_ADD: TableDefinition<&[u8], ()> = TableDefinition::new("log_add");
/// `(t_ret, eid)` of every retracted statement.
pub const LOG_RET: TableDefinition<&[u8], ()> = TableDefinition::new("log_ret");
/// Predicates that ever held two eids with the same `(s, p, o)`.
pub const PRED_MULTI: TableDefinition<i64, ()> = TableDefinition::new("pred_multi");
/// Predicate → live statements, distinct live subjects, distinct live objects.
pub const PRED_STATS: TableDefinition<i64, [u8; 24]> = TableDefinition::new("pred_stats");
/// `(s, key)` → `(value, updated_at)`.
pub const VOLATILE: TableDefinition<&[u8], [u8; 16]> = TableDefinition::new("volatile");
/// `(model, eid)` → `t`, sign bits and packed vector.
pub const EMBEDDING: TableDefinition<&[u8], &[u8]> = TableDefinition::new("embedding");
/// Indexed string value → token count and language.
pub const TEXT_DOC: TableDefinition<i64, &[u8]> = TableDefinition::new("text_doc");
/// Token → number of indexed values holding it.
pub const TEXT_TOKEN: TableDefinition<&str, u64> = TableDefinition::new("text_token");
/// `token 0x00 value` → token positions.
pub const TEXT_POST: TableDefinition<&[u8], &[u8]> = TableDefinition::new("text_post");
/// A live permutation table: key `(a, b, c, eid)`, value `t_add, v_from, v_to`.
pub type LiveTable = TableDefinition<'static, &'static [u8], [u8; 24]>;
/// A history permutation table: key `(a, b, c, MAX - t_add, eid)`.
pub type HistTable = TableDefinition<'static, &'static [u8], [u8; 32]>;

/// The six live permutations.
pub const LIVE: [(LiveTable, [usize; 3]); 6] = [
    (TableDefinition::new("l_spo"), [0, 1, 2]),
    (TableDefinition::new("l_sop"), [0, 2, 1]),
    (TableDefinition::new("l_pso"), [1, 0, 2]),
    (TableDefinition::new("l_pos"), [1, 2, 0]),
    (TableDefinition::new("l_osp"), [2, 0, 1]),
    (TableDefinition::new("l_ops"), [2, 1, 0]),
];

/// The three history permutations (all statements, newest addition first).
pub const HIST: [(HistTable, [usize; 3]); 3] = [
    (TableDefinition::new("h_spo"), [0, 1, 2]),
    (TableDefinition::new("h_pos"), [1, 2, 0]),
    (TableDefinition::new("h_osp"), [2, 0, 1]),
];

/// Index into [`LIVE`] of the permutation whose key starts with exactly the
/// bound positions `(s, p, o)`.
// @tg: implements:: [[openspec:redb-storage#Permutation layout]] {since: "2026-10-27"}
pub fn live_for(s: bool, p: bool, o: bool) -> usize {
    match (s, p, o) {
        (true, true, _) | (true, false, false) | (false, false, false) => 0,
        (true, false, true) => 1,
        (false, true, false) => 2,
        (false, true, true) => 3,
        (false, false, true) => 4,
    }
}

/// Index into [`HIST`] for the bound positions.
pub fn hist_for(s: bool, p: bool, o: bool) -> usize {
    match (s, p, o) {
        (true, true, _) | (true, false, false) | (false, false, false) => 0,
        (false, true, _) => 1,
        (_, false, true) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every combination of bound positions is a key prefix of the chosen
    /// permutation, so a scan reads one contiguous key range.
    // @tg: verifies:: [[openspec:redb-storage#Permutation layout#Every bound subset is a prefix scan]] {since: "2026-10-27", level: unit}
    #[test]
    fn every_bound_subset_is_a_prefix() {
        for mask in 0..8u8 {
            let bound = [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0];
            let n = bound.iter().filter(|b| **b).count();
            for (order, i) in [
                (LIVE[live_for(bound[0], bound[1], bound[2])].1, "live"),
                (HIST[hist_for(bound[0], bound[1], bound[2])].1, "history"),
            ] {
                let prefix: Vec<usize> = order[..n].to_vec();
                assert!(
                    prefix.iter().all(|p| bound[*p]),
                    "{i} permutation {order:?} does not start with the bound positions {bound:?}"
                );
            }
        }
    }
}
