//! The vocabulary configuration statements: `@vocab` and the prefix table. A port
//! of `tm_core::engine::config`.

use tm_core::mapping::BUILTIN_PREFIXES;
use tm_core::vocab;
use tm_core::{Error, ObjectId, Result, Valid, Value};

use super::Tx;

fn valid_prefix_name(n: &str) -> bool {
    let mut cs = n.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic())
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Tx<'_> {
    /// Sets `@vocab`: retracts the live `(sys:db sys:vocab …)` statement and asserts
    /// the new one. Stored data is not rewritten.
    pub fn set_vocab(&mut self, iri: &str) -> Result<()> {
        let db = self.encode(Value::iri(vocab::SYS_DB))?;
        let p = self.encode(Value::iri(vocab::SYS_VOCAB))?;
        self.retract_matching(Some(db), Some(p), None)?;
        self.assert(db, p, Value::iri(iri), Valid::ALWAYS)?;
        Ok(())
    }

    /// Declares (or redeclares) prefix `name` for `iri`. Built-in names fail with
    /// [`Error::ReservedNamespace`].
    pub fn set_prefix(&mut self, name: &str, iri: &str) -> Result<()> {
        if BUILTIN_PREFIXES.contains(&name) {
            return Err(Error::ReservedNamespace(format!("prefix `{name}`")));
        }
        if !valid_prefix_name(name) {
            return Err(Error::invalid_query(format!(
                "invalid prefix name `{name}`"
            )));
        }
        let sys = |n: &str| Value::iri(format!("{}{n}", vocab::SYS));
        let db = self.encode(Value::iri(vocab::SYS_DB))?;
        let p_prefix = self.encode(sys("prefix"))?;
        let p_name = self.encode(sys("prefixName"))?;
        let p_iri = self.encode(sys("prefixIri"))?;
        let existing: Option<ObjectId> = match self.lookup(&Value::str(name))? {
            Some(nid) => self
                .live_rows(None, Some(p_name), Some(nid))?
                .first()
                .map(|r| ObjectId::from_raw(r.s)),
            None => None,
        };
        let node = match existing {
            Some(n) => {
                self.retract_matching(Some(n), Some(p_iri), None)?;
                n
            }
            None => {
                let n = self.new_bnode()?;
                self.assert(db, p_prefix, n, Valid::ALWAYS)?;
                self.assert(n, p_name, Value::str(name), Valid::ALWAYS)?;
                n
            }
        };
        self.assert(node, p_iri, Value::iri(iri), Valid::ALWAYS)?;
        Ok(())
    }
}
