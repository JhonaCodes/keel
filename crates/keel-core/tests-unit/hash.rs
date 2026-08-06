// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `hash` (relocated out of src; included via #[path] in src/hash.rs).

    use super::*;
    use std::collections::BTreeMap;

    /// Determinism: same logical content → same hash (invariant 9).
    #[test]
    fn hash_is_deterministic_and_key_order_independent() {
        let mut a = BTreeMap::new();
        a.insert("z", 1);
        a.insert("a", 2);
        let h1 = ContentHash::of_canonical(&a).unwrap();
        let h2 = ContentHash::of_canonical(&a).unwrap();
        assert_eq!(h1, h2);

        // The same object expressed as JSON with a different textual key order
        // produces the same hash: canonicalization rules.
        let v1: serde_json::Value = serde_json::from_str(r#"{"a":2,"z":1}"#).unwrap();
        let v2: serde_json::Value = serde_json::from_str(r#"{"z":1,"a":2}"#).unwrap();
        assert_eq!(
            ContentHash::of_canonical(&v1).unwrap(),
            ContentHash::of_canonical(&v2).unwrap()
        );
    }

    #[test]
    fn display_and_parse_roundtrip() {
        let h = ContentHash::of_canonical(&"x").unwrap();
        let shown = h.to_string();
        assert!(shown.starts_with("sha256:"));
        assert_eq!(ContentHash::parse(&shown), Some(h));
    }
