//! Sort keys: decoded value order through `tm_sortkey` / `tm_vkey`, never raw ids;
//! missing values first under `Unbound`, last under `Null3VL` (design D11).

use tm_core::Result;

use super::{Gen, Rel};
use crate::plan::PKey;

impl Gen<'_> {
    /// The key expressions and their direction.
    pub fn sort_key_exprs(&mut self, keys: &[PKey], rel: &Rel) -> Result<Vec<(String, bool)>> {
        let mut out = Vec::new();
        for k in keys {
            let v = self.value(&k.expr, rel)?;
            let x = self.val_sql(&v);
            out.push((self.key_of(&v, &x), k.desc));
        }
        Ok(out)
    }

    /// ORDER BY terms for key expressions.
    pub fn order_terms(&self, keys: &[(String, bool)]) -> Vec<String> {
        keys.iter()
            .map(|(e, d)| format!("{e} {}", self.nulls(*d)))
            .collect()
    }

    /// ORDER BY terms for sort keys.
    pub fn sort_keys(&mut self, keys: &[PKey], rel: &Rel) -> Result<Vec<String>> {
        let e = self.sort_key_exprs(keys, rel)?;
        Ok(self.order_terms(&e))
    }
}
