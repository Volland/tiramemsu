## ADDED Requirements

### Requirement: Same logical database
A redb database SHALL store the same logical rows as a SQLite database with the same history: statement eids, `(s, p, o)`, transaction times, retraction kinds, valid intervals, term ids and contents, transaction instants, counters (including burned ids), `pred_multi`, volatile state and embeddings.

#### Scenario: Same history on both backends
- **WHEN** the same sequence of transactions runs on a new SQLite file and on a new redb file
- **THEN** both report the same eids and transaction numbers and `View::triples` on the history view returns identical rows

### Requirement: Permutation layout
The redb layout SHALL keep live statements in all six orders of `(s, p, o)` and all statements in the `spo`, `pos` and `osp` orders with the newest addition first, so that every combination of bound positions is answered by a key-prefix range scan.

#### Scenario: Every bound subset is a prefix scan
- **WHEN** live statements are scanned with each of the eight combinations of bound positions
- **THEN** each scan reads only keys within one prefix range and visits no key outside its result

#### Scenario: Retraction leaves the live tables
- **WHEN** a statement is retracted
- **THEN** none of the six live tables holds its eid and the three history tables still do, with `t_ret` set

### Requirement: Exact predicate statistics
For every predicate, the redb store SHALL keep the number of live statements, of distinct live subjects and of distinct live objects, updated in the same write transaction as the data.

#### Scenario: Statistics follow writes
- **WHEN** a transaction asserts `(a p 1)`, `(a p 2)` and `(b p 1)` and a later one retracts `(a p 2)`
- **THEN** after the first commit `p` has 3 live, 2 subjects and 2 objects, and after the second 2 live, 2 subjects and 1 object

#### Scenario: Failed transactions leave statistics unchanged
- **WHEN** a transaction asserts statements and then fails
- **THEN** the statistics equal those before it

### Requirement: Text index parity
On a redb database, text recall SHALL return the same hits in the same order as on a SQLite database with the same history, and its index SHALL be updated inside the writing transaction.

#### Scenario: Same recall on both backends
- **WHEN** the text-recall test corpus is loaded into both backends and every recall query of the test suite runs on both
- **THEN** the hits are equal and in the same order, and scores agree within a relative tolerance of 1e-9

#### Scenario: Speculation rolls the index back
- **WHEN** a speculation asserts a string and recall runs after it
- **THEN** recall does not find the string

### Requirement: Readers without a pool
Each view on a redb database SHALL read one snapshot without taking a pooled connection, so the number of concurrent views is not limited by a pool, and a view SHALL never wait for the writer.

#### Scenario: Many readers during a write
- **WHEN** 32 threads each open a view and scan while another thread holds an open write transaction
- **THEN** every scan completes without waiting for the writer and sees the last committed state

### Requirement: Lossless conversion
`convert(src, dst, backend)` SHALL copy a database to a new file of the other backend so that every row, counter and derived index is equal, numbering continues where the source stopped, and an existing destination is refused.

#### Scenario: Round trip
- **WHEN** a SQLite database with retractions, supersedes, burned ids, graphs, volatile state and embeddings is converted to redb and back to SQLite
- **THEN** the final file has the same rows and counters as the original, and the next assert on each of the three files gets the same eid

#### Scenario: Existing destination
- **WHEN** the destination file exists
- **THEN** `convert` fails and the destination is unchanged

### Requirement: Compaction
`Db::compact()` on a redb database SHALL reclaim free pages without changing any row, and SHALL fail with `Busy` while another view or transaction is open.

#### Scenario: Compaction after churn
- **WHEN** a database with heavy churn is compacted
- **THEN** its file is not larger than before and all views return the same rows
