use crossterm::event::Event;
use jaq_core::{load, Compiler, Ctx, Native, RcIter};
use jaq_json::Val;
use regex_lite::Regex;
use std::sync::Arc;
use std::time::Instant;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::models::IndexRate;

/// How long the cursor stays solid before blinking off, and vice versa.
/// Driven by our own clock rather than the terminal's SGR blink attribute,
/// which most modern terminal emulators ignore.
const CURSOR_BLINK_MS: u128 = 530;

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

/// A compiled, ready-to-run filter.
///
/// Regex mode deliberately never touches jq or JSON: it matches `.name`
/// directly with a pre-compiled `Regex`. This matters at scale — jq's
/// `test()` builtin recompiles its regex argument from scratch on every
/// single call (see jaq-std's `re()`), so routing plain name search through
/// `select(.name | test(...))` would mean recompiling the same regex once
/// per index on every redraw. Against a cluster with thousands of indices,
/// redrawn ~20x/second, that's tens of thousands of regex compilations a
/// second for a filter that never changes between keystrokes.
enum Compiled {
    Regex(Regex),
    Jq(CompiledFilter),
}

#[derive(Default)]
pub struct FilterState {
    pub active: bool,
    pub mode: FilterMode,
    pub input: Input,
    pub error: Option<String>,
    /// When the cursor last moved or the text last changed. Read by
    /// `cursor_visible()` to drive the blink animation; `None` renders as
    /// permanently visible (used before the filter is ever entered).
    cursor_blink_anchor: Option<Instant>,
    /// Cached compiled filter - only recompiled when input or mode changes
    compiled: Option<Compiled>,
}

impl FilterState {
    pub fn enter(&mut self) {
        self.active = true;
        self.cursor_blink_anchor = Some(Instant::now());
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

    /// Feeds a raw key event into the input box. Cursor-only movement
    /// (arrows, Home/End, word-jumps) never touches `recompile()` — only
    /// an actual change to the text does — so navigating the filter box
    /// stays instant regardless of how expensive the current filter is to
    /// compile. Also resets the blink phase so the caret is solid right
    /// after any edit or movement, blinking again only once idle.
    pub fn handle_key(&mut self, event: &Event) {
        let Some(changed) = self.input.handle_event(event) else {
            return;
        };
        self.cursor_blink_anchor = Some(Instant::now());
        if changed.value {
            self.recompile();
        }
    }

    /// True during the "on" phase of the cursor blink cycle.
    pub fn cursor_visible(&self) -> bool {
        match self.cursor_blink_anchor {
            Some(anchor) => (anchor.elapsed().as_millis() / CURSOR_BLINK_MS) % 2 == 0,
            None => true,
        }
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

        match self.mode {
            // Compiled once here and reused for every index — see the
            // `Compiled` doc comment for why this must not go through jq.
            FilterMode::Regex => match Regex::new(text) {
                Ok(re) => {
                    self.error = None;
                    self.compiled = Some(Compiled::Regex(re));
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                    self.compiled = None;
                }
            },
            // The user types the boolean expression only; `select(...)` is
            // implicit so they never need to know jq's select syntax.
            FilterMode::Jq => {
                let program = format!("select({})", text);
                match compile_filter(&program) {
                    Ok(filter) => {
                        let filter = Arc::new(filter);
                        // jq compiles a syntactically valid program even when
                        // it will always fail at runtime — e.g. a type error
                        // in the expression. Probe it against a dummy index
                        // now so errors surface immediately instead of only
                        // once real data arrives.
                        let probe = serde_json::json!({
                            "name": "", "doc_count": 0, "rate_per_sec": 0.0,
                            "size_bytes": 0, "health": "",
                        });
                        match evaluate(&filter, Val::from(probe)) {
                            Ok(_) => {
                                self.error = None;
                                self.compiled = Some(Compiled::Jq(filter));
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
        }
    }

    pub fn is_match(&self, index: &IndexRate) -> bool {
        match &self.compiled {
            // No filter or error means match everything
            None => true,
            Some(Compiled::Regex(re)) => re.is_match(&index.name),
            Some(Compiled::Jq(filter)) => match serde_json::to_value(index) {
                Ok(json) => evaluate(filter, Val::from(json)).unwrap_or(false),
                Err(_) => true,
            },
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

    fn mock_index(name: &str, doc_count: u64, rate_per_sec: f64) -> IndexRate {
        IndexRate {
            name: name.to_string(),
            doc_count,
            rate_per_sec,
            size_bytes: 0,
            health: "green".to_string(),
        }
    }

    fn named(name: &str) -> IndexRate {
        mock_index(name, 0, 0.0)
    }

    #[test]
    fn test_filter_compilation() {
        let mut filter = FilterState::default();

        // Empty filter matches everything
        filter.recompile();
        assert!(filter.error.is_none());
        assert!(filter.is_match(&named("any-index")));

        // Default mode is a plain regex against .name — no jq needed
        assert_eq!(filter.mode, FilterMode::Regex);
        filter.input = "my-test-index".into();
        filter.recompile();
        if let Some(ref e) = filter.error {
            eprintln!("Error compiling filter: {}", e);
        }
        assert!(filter.error.is_none());
        assert!(filter.is_match(&named("my-test-index")));
        assert!(!filter.is_match(&named("other-index")));

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
        assert!(filter.is_match(&mock_index("idx", 2000, 0.0)));
        assert!(!filter.is_match(&mock_index("idx", 500, 0.0)));

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
            mode: FilterMode::Jq,
            input: ".doc_count > 1000".into(),
            ..Default::default()
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&mock_index("idx", 2000, 0.0)));
        assert!(!filter_state.is_match(&mock_index("idx", 500, 0.0)));
    }

    #[test]
    fn test_filter_string_contains() {
        let mut filter_state = FilterState {
            mode: FilterMode::Jq,
            input: ".name | contains(\"test\")".into(),
            ..Default::default()
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&named("my-test-index")));
        assert!(!filter_state.is_match(&named("production-index")));
    }

    #[test]
    fn test_filter_regex_matches_name_substring() {
        // Regex mode matches anywhere in the name, like a search box
        let mut filter_state = FilterState {
            input: "idx-[0-9]+".into(),
            ..Default::default()
        };
        filter_state.recompile();

        assert!(filter_state.is_match(&named("my-idx-42")));
        assert!(!filter_state.is_match(&named("my-index")));
    }

    #[test]
    fn test_cursor_movement_skips_recompile() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut filter = FilterState {
            input: "abc".into(),
            ..Default::default()
        };
        // Plant a sentinel that a real recompile of "abc" (a valid regex)
        // would definitely overwrite, so if it survives a cursor move we
        // know recompile() was never called for it.
        filter.error = Some("SENTINEL".to_string());
        filter.compiled = None;

        let left = Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        filter.handle_key(&left);

        assert_eq!(filter.error.as_deref(), Some("SENTINEL"));
        assert!(filter.compiled.is_none());

        // Typing an actual character, on the other hand, must still
        // recompile and clear the sentinel.
        let c = Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        filter.handle_key(&c);
        assert!(filter.error.is_none());
        assert!(filter.compiled.is_some());
    }

    #[test]
    fn test_filter_regex_compiled_once_not_per_match() {
        // Regression guard for the perf bug where regex mode routed through
        // jq's test(), which recompiles its regex argument on every call.
        // Compiling here and reusing across many matches should stay fast
        // even though this doesn't directly measure jq's internals.
        let mut filter_state = FilterState {
            input: "idx-[0-9]+".into(),
            ..Default::default()
        };
        filter_state.recompile();

        for i in 0..10_000 {
            let name = format!("idx-{i}");
            assert!(filter_state.is_match(&mock_index(&name, 0, 0.0)));
        }
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
            let _ = filter_state.is_match(&mock_index("idx", i, 0.0));
        }
    }
}
