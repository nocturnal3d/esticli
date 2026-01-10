use jaq_core::{load, Compiler, Ctx, Native, RcIter};
use jaq_json::Val;
use serde::Serialize;
use std::sync::Arc;
use tui_input::Input;

/// Compiled filter that can be reused across multiple matches
type CompiledFilter = Arc<jaq_core::Filter<Native<Val>>>;

#[derive(Default)]
pub struct FilterState {
    pub active: bool,
    pub input: Input,
    pub error: Option<String>,
    /// Cached compiled filter - only recompiled when input changes
    compiled: Option<CompiledFilter>,
}

impl FilterState {
    pub fn enter(&mut self) {
        self.active = true;
    }

    pub fn exit(&mut self) {
        self.active = false;
    }

    pub fn clear(&mut self) {
        self.input.reset();
        self.error = None;
        self.compiled = None;
        self.active = false;
    }

    pub fn recompile(&mut self) {
        let text = self.input.value();
        if text.is_empty() {
            self.error = None;
            self.compiled = None;
        } else {
            match compile_filter(text) {
                Ok(filter) => {
                    self.error = None;
                    self.compiled = Some(Arc::new(filter));
                }
                Err(e) => {
                    self.error = Some(e);
                    self.compiled = None;
                }
            }
        }
    }

    pub fn is_match<T: Serialize>(&self, item: &T) -> bool {
        // No filter or error means match everything
        let Some(filter) = &self.compiled else {
            return true;
        };

        match serde_json::to_value(item) {
            Ok(json) => {
                // Run the pre-compiled filter
                let inputs = RcIter::new(core::iter::empty());
                let val = Val::from(json);
                let mut results = filter.run((Ctx::new([], &inputs), val));

                // For select() filters, a match produces output; no match produces nothing
                results.next().is_some()
            }
            Err(_) => true,
        }
    }
}

/// Compile a jq filter expression (called once when filter text changes)
fn compile_filter(filter_str: &str) -> Result<jaq_core::Filter<Native<Val>>, String> {
    // Create the program
    let program = load::File {
        code: filter_str,
        path: (),
    };

    // Load with standard library definitions
    let loader = load::Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let arena = load::Arena::default();

    let modules = loader.load(&arena, program).map_err(|errs| {
        errs.into_iter()
            .map(|e| format!("{:?}", e.1))
            .collect::<Vec<_>>()
            .join(", ")
    })?;

    // Compile with standard library functions
    let filter = Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|errs| {
            errs.into_iter()
                .map(|e| format!("{:?}", e.1))
                .collect::<Vec<_>>()
                .join(", ")
        })?;

    Ok(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_compilation() {
        let mut filter = FilterState::default();

        // Empty filter matches everything
        filter.recompile();
        assert!(filter.error.is_none());
        assert!(filter.is_match(&serde_json::json!({"name": "any-index"})));

        // Valid jq filter - select by exact name
        filter.input = "select(.name == \"my-test-index\")".into();
        filter.recompile();
        if let Some(ref e) = filter.error {
            eprintln!("Error compiling filter: {}", e);
        }
        assert!(filter.error.is_none());
        assert!(filter.is_match(&serde_json::json!({"name": "my-test-index"})));
        assert!(!filter.is_match(&serde_json::json!({"name": "other-index"})));

        // Invalid jq syntax
        filter.input = "invalid query syntax {{".into();
        filter.recompile();
        assert!(filter.error.is_some());
    }

    #[test]
    fn test_filter_clear() {
        let mut filter = FilterState::default();
        filter.input = "select(.name == \"test\")".into();
        filter.recompile();
        filter.enter();

        assert!(filter.active);
        assert!(!filter.input.value().is_empty());

        filter.clear();
        assert!(!filter.active);
        assert!(filter.input.value().is_empty());
    }

    #[test]
    fn test_filter_numeric_comparison() {
        let mut filter_state = FilterState {
            active: false,
            input: "select(.doc_count > 1000)".into(),
            error: None,
            compiled: None,
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&serde_json::json!({"doc_count": 2000})));
        assert!(!filter_state.is_match(&serde_json::json!({"doc_count": 500})));
    }

    #[test]
    fn test_filter_string_contains() {
        let mut filter_state = FilterState {
            active: false,
            input: "select(.name | contains(\"test\"))".into(),
            error: None,
            compiled: None,
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&serde_json::json!({"name": "my-test-index"})));
        assert!(!filter_state.is_match(&serde_json::json!({"name": "production-index"})));
    }

    #[test]
    fn test_filter_performance() {
        // Verify that multiple matches reuse the compiled filter
        let mut filter_state = FilterState::default();
        filter_state.input = "select(.doc_count > 100)".into();
        filter_state.recompile();

        // This should be fast since filter is pre-compiled
        for i in 0..1000 {
            let _ = filter_state.is_match(&serde_json::json!({"doc_count": i}));
        }
    }
}
