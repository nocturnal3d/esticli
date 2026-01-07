use serde::Serialize;
use tellaro_query_language::Tql;
use tui_input::Input;

#[derive(Default)]
pub struct FilterState {
    pub active: bool,
    pub input: Input,
    pub error: Option<String>,
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
        self.active = false;
    }

    pub fn recompile(&mut self) {
        let text = self.input.value();
        if text.is_empty() {
            self.error = None;
        } else {
            // Validate query by trying to parse it.
            // Argument order: tql.query(records, query_string)
            let dummy = serde_json::json!({});
            match Tql::new().query(&[dummy], text) {
                Ok(_) => {
                    self.error = None;
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                }
            }
        }
    }

    pub fn is_match<T: Serialize>(&self, item: &T) -> bool {
        let text = self.input.value();
        if text.is_empty() || self.error.is_some() {
            return true;
        }

        match serde_json::to_value(item) {
            Ok(json) => {
                // TQL usually returns a filtered list. If our single item remains, it matches.
                match Tql::new().query(&[json], text) {
                    Ok(results) => !results.is_empty(),
                    Err(_) => true,
                }
            }
            Err(_) => true,
        }
    }
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

        // Valid TQL
        filter.input = "name = 'my-test-index'".into();
        filter.recompile();
        assert!(filter.error.is_none());
        assert!(filter.is_match(&serde_json::json!({"name": "my-test-index"})));
        assert!(!filter.is_match(&serde_json::json!({"name": "other-index"})));

        // Invalid TQL
        filter.input = "invalid query".into();
        filter.recompile();
        assert!(filter.error.is_some());
    }

    #[test]
    fn test_filter_clear() {
        let mut filter = FilterState::default();
        filter.input = "name = 'test'".into();
        filter.recompile();
        filter.enter();

        assert!(filter.active);
        assert!(!filter.input.value().is_empty());

        filter.clear();
        assert!(!filter.active);
        assert!(filter.input.value().is_empty());
    }
}
