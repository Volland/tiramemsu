## ADDED Requirements

### Requirement: Metrics snapshot
`Db::metrics()` SHALL return a snapshot of counters kept since the database was opened: writer commits, dry runs and writer-lock waits (count, total and maximum wait); maintenance (`PRAGMA optimize` and `ANALYZE` runs and their total time, text-index catch-up runs and time, whether statistics are due); reader-pool size, acquisitions, waits (count, total and maximum wait) and timeouts; term-cache hits, misses, clears or evictions, length and capacity, for the writer dictionary and the reader cache separately; and the storage facts format version, `last_t`, page size, page count and freelist page count. Durations SHALL be in microseconds and counters SHALL never decrease while the database is open. The snapshot SHALL carry a layout version.

#### Scenario: Commits and maintenance are counted
- **WHEN** a database opened with `optimize_every = 2` commits four write transactions
- **THEN** `metrics().writer.commits` is 4 and `metrics().maintenance.optimize_runs` equals `Db::optimize_runs()` and is at least 2

#### Scenario: Reader wait is counted
- **WHEN** every reader of the pool is busy and another read waits 20 ms for one
- **THEN** `readers.waits` has increased by 1 and `readers.wait_us_total` by at least 20 000

#### Scenario: Reader timeout is counted
- **WHEN** a read fails with `PoolTimeout`
- **THEN** `readers.timeouts` has increased by 1

#### Scenario: Term cache hit
- **WHEN** the same IRI is decoded twice through a reader with the cache enabled
- **THEN** `term_cache.readers.hits` has increased by at least 1

### Requirement: Metrics are cheap and non-blocking
Taking a snapshot SHALL NOT take the writer lock, SHALL NOT wait for a reader and SHALL run no SQL other than constant-time `PRAGMA` reads for the storage facts. It SHALL never scan the triple, term or transaction tables.

#### Scenario: Snapshot during a write
- **WHEN** a write transaction holds the writer and another thread calls `metrics()`
- **THEN** the call returns without waiting for the transaction to finish

### Requirement: Metrics in the JSON bridge and bindings
The JSON bridge SHALL expose the snapshot as the read op `metrics`, and the Node and Python bindings SHALL expose it as `Db.metrics()`, with the same field names in camelCase on the bridge and Node and snake_case in Python.

#### Scenario: Same snapshot shape everywhere
- **WHEN** `metrics` is called through the bridge, Node and Python on one database
- **THEN** each result contains the writer, maintenance, readers, term cache and storage sections with the same counters
