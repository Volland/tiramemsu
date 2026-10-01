## MODIFIED Requirements

### Requirement: Engine metadata counters
The `meta` table SHALL hold exactly the integer keys `format_version`, `next_term`, `next_node`, `next_bnode`, `next_stmt`, `last_t`, `last_instant` and `multi_version`. A fresh database SHALL start with `format_version = 1`, `last_t = 0`, `last_instant = 0`, `multi_version = 0`, and every `next_*` counter at 1. Every id the engine allocates SHALL come from the corresponding counter and never from the maximum existing rowid, and each counter SHALL only ever increase. The largest `NODE`, `BNODE`, `STMT` or `TX` number the engine ever allocates SHALL be 2⁴⁸ − 1: once a counter has handed out 2⁴⁸ − 1 (so `next_node`, `next_bnode` or `next_stmt` equals 2⁴⁸, or `last_t` equals 2⁴⁸ − 1), the next allocation of that kind SHALL fail the transaction with `IdSpaceExhausted { kind }` and leave no trace. `next_*` therefore never exceeds 2⁴⁸ and `last_t` never exceeds 2⁴⁸ − 1.

#### Scenario: Fresh counters
- **WHEN** a database is freshly created
- **THEN** `meta` contains `format_version = 1`, `next_term = 1`, `next_node = 1`, `next_bnode = 1`, `next_stmt = 1`, `last_t = 0`, `last_instant = 0` and `multi_version = 0`

#### Scenario: Counters advance with allocation
- **WHEN** a committed transaction creates 3 statements and 1 new node
- **THEN** `next_stmt` has advanced by at least 3, `next_node` by at least 1, and `last_t` and `last_instant` equal the new transaction's number and instant

#### Scenario: Ids are not derived from existing rows
- **WHEN** `next_stmt` is ahead of the largest stored statement id (because earlier ids were burned by a speculative transaction)
- **THEN** the next statement id allocated equals `next_stmt`, not the largest stored id plus one

#### Scenario: Last counter value is allocated
- **WHEN** `next_stmt` equals 2⁴⁸ − 1 and a transaction asserts a new statement
- **THEN** the statement gets number 2⁴⁸ − 1 with origin 0, and `next_stmt` becomes 2⁴⁸

#### Scenario: Counter bound
- **WHEN** `next_stmt` equals 2⁴⁸ (statement 2⁴⁸ − 1 has been allocated) and a transaction asserts a new statement
- **THEN** the transaction fails with `IdSpaceExhausted { kind: STMT }` and no tx row, triple or term is written
