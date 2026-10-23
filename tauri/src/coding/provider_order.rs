//! Rebuilding a provider map in a new key order, shared by every CLI whose
//! provider list is drag-sortable.
//!
//! Pi (`models.json`), Oh My Pi (`models.yml`) and dsh (`settings.yaml`) all
//! store providers as an ordered map whose **key order is the display order**,
//! and all three back their drag handle with a `reorder_*_providers` command.
//! The rebuild rule therefore has to be identical: a key the payload mentions
//! moves, a key it does not mention keeps its relative position behind the
//! listed ones, and an unknown or duplicated key is ignored.
//!
//! Keeping one copy is what makes that true. The three commands used to inline
//! this loop; a later fix to (say) where unlisted keys land would have been
//! applied to one of them and silently not to the other two.
//!
//! **Do not use `BTreeMap` to read these maps back.** `serde_json` runs with
//! `preserve_order`, so the `Map` already keeps insertion order; rebuilding it
//! by sorted key would turn every drag back into alphabetical order.

use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Rebuild `object` in `ordered_keys` order, keeping every entry.
///
/// Keys the payload does not mention keep their relative order **after** the
/// listed ones: a reorder writes the order it saw, and any key added by another
/// writer between load and drop must not be dropped or moved.
pub(crate) fn reorder_map_by_keys(
    object: &Map<String, Value>,
    ordered_keys: &[String],
) -> Map<String, Value> {
    let mut ordered = Map::with_capacity(object.len());
    let mut placed = BTreeSet::new();
    for key in ordered_keys {
        if let Some(value) = object.get(key) {
            if placed.insert(key.clone()) {
                ordered.insert(key.clone(), value.clone());
            }
        }
    }
    for (key, value) in object {
        if placed.insert(key.clone()) {
            ordered.insert(key.clone(), value.clone());
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object literal")
    }

    fn keys(value: &Map<String, Value>) -> Vec<&str> {
        value.keys().map(String::as_str).collect()
    }

    #[test]
    fn listed_keys_move_and_unlisted_entries_stay_behind_them() {
        let object = map(json!({ "a": { "n": 1 }, "b": {}, "c": {} }));
        let reordered = reorder_map_by_keys(&object, &["c".to_string(), "a".to_string()]);
        assert_eq!(keys(&reordered), vec!["c", "a", "b"]);
    }

    #[test]
    fn unknown_and_duplicate_keys_are_ignored() {
        let object = map(json!({ "a": {}, "b": {} }));
        let reordered = reorder_map_by_keys(
            &object,
            &["missing".to_string(), "b".to_string(), "b".to_string()],
        );
        assert_eq!(keys(&reordered), vec!["b", "a"]);
    }

    #[test]
    fn an_empty_payload_keeps_the_stored_order() {
        let object = map(json!({ "b": {}, "a": {} }));
        let reordered = reorder_map_by_keys(&object, &[]);
        assert_eq!(keys(&reordered), vec!["b", "a"]);
    }
}
