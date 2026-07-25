use jaq_core::{load, Compiler, Ctx, Native, RcIter};
use jaq_json::Val;
use serde::Serialize;
use std::sync::Arc;
use tui_input::Input;

/// Compiled filter that can be reused across multiple matches
type CompiledFilter = Arc<jaq_core::Filter<Native<Val>>>;

/// Which syntax the filter input box is currently interpreted as.
///
/// `/` opens the filter in `Regex` mode, where the raw input is a plain
/// regex matched against `.name` — no jq knowledge required. Pressing `/`
/// again while the input is still empty ("//") switches to `Jq` mode,
/// where the raw input is a jq boolean expression that gets wrapped in
/// `select(...)` automatically, so the user never types `select` themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    Regex,
    Jq,
}

impl FilterMode {
    fn toggled(self) -> Self {
        match self {
            FilterMode::Regex => FilterMode::Jq,
            FilterMode::Jq => FilterMode::Regex,
        }
    }
}

#[derive(Default)]
pub struct FilterState {
    pub active: bool,
    pub mode: FilterMode,
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
        self.mode = FilterMode::default();
        self.error = None;
        self.compiled = None;
        self.active = false;
    }

    /// Switches between regex and jq syntax. Only meaningful while the
    /// input is empty (a `/` typed after any text is just a character), so
    /// callers gate this on `input.value().is_empty()` before invoking it.
    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();
        self.recompile();
    }

    pub fn recompile(&mut self) {
        let text = self.input.value();
        if text.is_empty() {
            self.error = None;
            self.compiled = None;
            return;
        }

        let program = match self.mode {
            // Match the raw regex against the index name only. Quoting via
            // serde_json gives us correct jq/JSON string escaping for free.
            FilterMode::Regex => {
                let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
                format!("select(.name | test({}))", quoted)
            }
            // The user types the boolean expression only; `select(...)` is
            // implicit so they never need to know jq's select syntax.
            FilterMode::Jq => format!("select({})", text),
        };

        match compile_filter(&program) {
            Ok(filter) => {
                let filter = Arc::new(filter);
                // jq compiles a syntactically valid program even when it
                // will always fail at runtime — e.g. an invalid regex passed
                // to test() is just a string literal as far as the compiler
                // is concerned. Probe it against a dummy index now so
                // errors like that surface immediately instead of silently
                // matching nothing (or everything) once real data arrives.
                let probe = serde_json::json!({
                    "name": "", "doc_count": 0, "rate_per_sec": 0.0,
                    "size_bytes": 0, "health": "",
                });
                match evaluate(&filter, Val::from(probe)) {
                    Ok(_) => {
                        self.error = None;
                        self.compiled = Some(filter);
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.compiled = None;
                    }
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.compiled = None;
            }
        }
    }

    pub fn is_match<T: Serialize>(&self, item: &T) -> bool {
        // No filter or error means match everything
        let Some(filter) = &self.compiled else {
            return true;
        };

        match serde_json::to_value(item) {
            Ok(json) => evaluate(filter, Val::from(json)).unwrap_or(false),
            Err(_) => true,
        }
    }
}

/// Runs a compiled `select(...)` filter against a value: `Ok(true)` if it
/// matched, `Ok(false)` if it didn't, `Err` if the filter itself raised a
/// runtime error (e.g. an invalid regex).
fn evaluate(filter: &CompiledFilter, val: Val) -> Result<bool, String> {
    let inputs = RcIter::new(core::iter::empty());
    let mut results = filter.run((Ctx::new([], &inputs), val));
    match results.next() {
        Some(Ok(_)) => Ok(true),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(false),
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

        // Default mode is a plain regex against .name — no jq needed
        assert_eq!(filter.mode, FilterMode::Regex);
        filter.input = "my-test-index".into();
        filter.recompile();
        if let Some(ref e) = filter.error {
            eprintln!("Error compiling filter: {}", e);
        }
        assert!(filter.error.is_none());
        assert!(filter.is_match(&serde_json::json!({"name": "my-test-index"})));
        assert!(!filter.is_match(&serde_json::json!({"name": "other-index"})));

        // Invalid regex syntax
        filter.input = "unbalanced[bracket".into();
        filter.recompile();
        assert!(filter.error.is_some());
    }

    #[test]
    fn test_filter_toggle_mode() {
        let mut filter = FilterState::default();
        assert_eq!(filter.mode, FilterMode::Regex);

        // "//" (second '/' while input is empty) switches to jq mode
        filter.toggle_mode();
        assert_eq!(filter.mode, FilterMode::Jq);

        // A bare boolean expression gets wrapped in select(...) automatically
        filter.input = ".doc_count > 1000".into();
        filter.recompile();
        assert!(filter.error.is_none());
        assert!(filter.is_match(&serde_json::json!({"doc_count": 2000})));
        assert!(!filter.is_match(&serde_json::json!({"doc_count": 500})));

        // Toggling back switches to regex mode again
        filter.toggle_mode();
        assert_eq!(filter.mode, FilterMode::Regex);
    }

    #[test]
    fn test_filter_clear() {
        let mut filter = FilterState {
            input: "test".into(),
            ..Default::default()
        };
        filter.recompile();
        filter.enter();
        filter.toggle_mode();

        assert!(filter.active);
        assert!(!filter.input.value().is_empty());

        filter.clear();
        assert!(!filter.active);
        assert!(filter.input.value().is_empty());
        assert_eq!(filter.mode, FilterMode::Regex);
    }

    #[test]
    fn test_filter_numeric_comparison() {
        let mut filter_state = FilterState {
            active: false,
            mode: FilterMode::Jq,
            input: ".doc_count > 1000".into(),
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
            mode: FilterMode::Jq,
            input: ".name | contains(\"test\")".into(),
            error: None,
            compiled: None,
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&serde_json::json!({"name": "my-test-index"})));
        assert!(!filter_state.is_match(&serde_json::json!({"name": "production-index"})));
    }

    #[test]
    fn test_filter_regex_matches_name_substring() {
        // Regex mode matches anywhere in the name, like a search box
        let mut filter_state = FilterState {
            input: "idx-[0-9]+".into(),
            ..Default::default()
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&serde_json::json!({"name": "my-idx-42"})));
        assert!(!filter_state.is_match(&serde_json::json!({"name": "my-index"})));
    }

    #[test]
    fn test_filter_performance() {
        // Verify that multiple matches reuse the compiled filter
        let mut filter_state = FilterState {
            mode: FilterMode::Jq,
            input: ".doc_count > 100".into(),
            ..Default::default()
        };
        filter_state.recompile();

        // This should be fast since filter is pre-compiled
        for i in 0..1000 {
            let _ = filter_state.is_match(&serde_json::json!({"doc_count": i}));
        }
    }
}
