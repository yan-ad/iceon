//! Counts describe the loaded snapshot, never an inferred server-side total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InventoryCount {
    filtered: usize,
    loaded: usize,
    partial: bool,
}
impl InventoryCount {
    pub(crate) fn new(filtered: usize, loaded: usize, partial: bool) -> Self {
        Self {
            filtered,
            loaded,
            partial,
        }
    }
    pub(crate) fn label(self) -> String {
        let suffix = if self.partial { "+" } else { "" };
        if self.filtered == self.loaded {
            format!("{}{suffix} items", self.loaded)
        } else {
            format!("{} / {}{suffix} items", self.filtered, self.loaded)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::InventoryCount;
    #[test]
    fn loaded_filtered_and_partial_counts_are_explicit() {
        for (filtered, loaded, partial, expected) in [
            (12, 12, false, "12 items"),
            (3, 12, false, "3 / 12 items"),
            (12, 12, true, "12+ items"),
            (3, 12, true, "3 / 12+ items"),
            (0, 12, true, "0 / 12+ items"),
            (0, 0, false, "0 items"),
        ] {
            assert_eq!(
                InventoryCount::new(filtered, loaded, partial).label(),
                expected
            );
        }
    }
}
