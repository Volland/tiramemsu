//! The vocabulary configuration statements: `@vocab` and the prefix table
//! (`lat.md/data-model#Vocabulary Mapping`). Both are ordinary versioned statements
//! about `sys:db`; these helpers replace the previous live setting.

use super::Tx;
use crate::error::{Error, Result};
use crate::exec::SqlValue;
use crate::id::ObjectId;
use crate::report::Valid;
use crate::value::Value;
use crate::vocab;

/// Prefix names of the namespaces that cannot be redeclared.
pub const BUILTIN_PREFIXES: [&str; 6] = ["sys", "tm", "rdf", "rdfs", "xsd", "v"];

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
        // an existing live declaration of this name
        let existing: Option<ObjectId> = match self.lookup(&Value::str(name))? {
            Some(nid) => self
                .read_with(|e| {
                    e.query_i64(
                        "SELECT s FROM triple WHERE p = ?1 AND o = ?2 AND t_ret IS NULL ORDER BY eid LIMIT 1",
                        &[SqlValue::Integer(p_name.raw()), SqlValue::Integer(nid.raw())],
                    )
                })?
                .map(ObjectId::from_raw),
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
