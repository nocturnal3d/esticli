//! Change tracking for the state the visible-index cache is derived from.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh version number, unique across the whole process.
///
/// Unique rather than merely increasing per value: replacing a value outright
/// (`app.indices = rows.into()`) must never hand the replacement a version
/// that an earlier value already had, or a cache keyed on it would mistake
/// the new contents for the old.
pub fn next_version() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A value that gets a new version on every mutable access.
///
/// Reads go through `Deref` and leave the version alone. Anything that could
/// change the value — pushing, sorting, assigning through an index, replacing
/// it — has to go through `DerefMut` or `From`, both of which take a new
/// version. A cache keyed on the version therefore cannot be left stale by a
/// mutation nobody told it about, including one made directly on a `pub`
/// field from a test. A mutable access that turns out to change nothing costs
/// one spurious recompute, never a wrong answer.
#[derive(Debug)]
pub struct Versioned<T> {
    value: T,
    version: u64,
}

impl<T> Versioned<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            version: next_version(),
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}

impl<T: Default> Default for Versioned<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> From<T> for Versioned<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for Versioned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Versioned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.version = next_version();
        &mut self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reads_keep_the_version_and_writes_change_it() {
        let mut rows = Versioned::new(vec![1, 2, 3]);
        let version = rows.version();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], 1);
        assert_eq!(rows.version(), version);

        rows[0] = 10;
        let after_index = rows.version();
        assert_ne!(after_index, version);

        rows.push(4);
        assert_ne!(rows.version(), after_index);
    }

    #[test]
    fn test_a_replacement_never_reuses_an_old_version() {
        let first = Versioned::new(vec![1]);
        let second: Versioned<Vec<i32>> = vec![1].into();
        assert_ne!(first.version(), second.version());
    }
}
