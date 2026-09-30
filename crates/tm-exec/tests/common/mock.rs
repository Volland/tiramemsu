//! A mock path operator: `tm_path(start, …)` returns the zero-hop path
//! `(start, start, 0, "[]")` and records every call's arguments.

use std::sync::{Arc, Mutex};

use tiramemsu::{NativeKind, NativeOperator, SqlValue};
use tm_core::{HostRegistry, TableFunction};

#[derive(Default)]
pub struct MockPath {
    pub calls: Arc<Mutex<Vec<Vec<SqlValue>>>>,
}

impl NativeOperator for MockPath {
    fn kind(&self) -> NativeKind {
        NativeKind::Path
    }

    fn tvf_name(&self) -> &'static str {
        "tm_path"
    }

    fn register(&self, host: &mut dyn HostRegistry) -> tm_core::Result<()> {
        let calls = self.calls.clone();
        host.register_table(TableFunction {
            name: "tm_path".to_string(),
            args: ["start", "path", "mode", "max_hops", "view"]
                .map(String::from)
                .to_vec(),
            columns: ["start", "end", "hops", "path_json"]
                .map(String::from)
                .to_vec(),
            func: Arc::new(move |args| {
                calls.lock().unwrap().push(args.to_vec());
                let start = args.first().cloned().unwrap_or(SqlValue::Null);
                if start.is_null() {
                    return Ok(Vec::new());
                }
                Ok(vec![vec![
                    start.clone(),
                    start,
                    SqlValue::Integer(0),
                    SqlValue::Text("[]".to_string()),
                ]])
            }),
        })
    }
}
