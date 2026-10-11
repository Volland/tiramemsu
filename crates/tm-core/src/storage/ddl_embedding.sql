-- Format 5: embeddings (OpenSpec change add-vector-recall).
-- Verbatim DDL of lat.md/storage.md#Embeddings. Vectors are source data supplied
-- by the caller, one per (model, statement): append-only, guarded like the graph.
-- Search is exact and computed in Rust, so no host capability is needed.

CREATE TABLE embedding (
  model INTEGER NOT NULL,  -- IRI ObjectId of the model
  eid   INTEGER NOT NULL,  -- STMT ObjectId of the statement the vector describes
  t     INTEGER NOT NULL,  -- transaction that stored it
  bits  BLOB NOT NULL,     -- sign bits, ceil(dim / 8) bytes, for the binary prefilter
  vec   BLOB NOT NULL      -- dim values, little-endian f32 or f16; unit length for cosine
) STRICT;

CREATE UNIQUE INDEX embedding_model_eid ON embedding(model, eid);
-- covering index for the binary prefilter: a prefilter scan reads sign bits from
-- the index (about 100 bytes per row) instead of each ~3 KB table row
CREATE INDEX embedding_bits ON embedding(model, eid, bits);

CREATE TRIGGER embedding_no_delete BEFORE DELETE ON embedding
BEGIN SELECT RAISE(ABORT, 'tiramemsu: embeddings are never deleted'); END;
CREATE TRIGGER embedding_no_update BEFORE UPDATE ON embedding
BEGIN SELECT RAISE(ABORT, 'tiramemsu: embeddings are immutable'); END;
