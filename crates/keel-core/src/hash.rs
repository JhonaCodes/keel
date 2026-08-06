// SPDX-License-Identifier: Apache-2.0
//! The ONLY canonical hashing authority in the system (invariant 9).
//!
//! WHY IT EXISTS: local and CI must verify THE SAME snapshot hash
//! (spec §4.9 inv. 9, ADR-007). If hashing were duplicated in the compiler
//! and in the CI plane, the invariant would be a hope, not a guarantee.
//! Every piece that needs to hash Keel content goes through here.
//!
//! CANONICALIZATION: serialization goes through `serde_json::Value`, whose
//! `Map` is backed by a `BTreeMap` (the `preserve_order` feature is NOT
//! enabled in this workspace), so object keys are always emitted in sorted
//! order. Same logical content → same bytes → same hash, on any platform.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// `sha256:<hex>` content hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes the canonical form (JSON with sorted keys) of any serializable
    /// value. It is the system's only hashing function.
    pub fn of_canonical<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        // Going through Value guarantees canonical key ordering even if the
        // source struct serializes its fields in declaration order.
        let canonical = serde_json::to_value(value)?;
        let bytes = serde_json::to_vec(&canonical)?;
        let digest = Sha256::digest(&bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(ContentHash(out))
    }

    /// Parses the `sha256:<hex>` presentation format.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix("sha256:")?;
        if hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            out[i] = (hi * 16 + lo) as u8;
        }
        Some(ContentHash(out))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:")?;
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

// Serialized as its presentation form ("sha256:…") so that snapshots and
// ledger entries are readable and interoperable (§11.6: the hash travels in
// SARIF properties).
impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ContentHash::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid content hash: {s}")))
    }
}

#[cfg(test)]
#[path = "../tests-unit/hash.rs"]
mod tests;