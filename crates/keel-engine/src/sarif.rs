// SPDX-License-Identifier: Apache-2.0
//! SARIF export — the normative findings format (spec section 11.6, ADR-016).
//!
//! Keel does not maintain a proprietary format: it extends SARIF via
//! `properties` (evidence class, rule, snapshot hash, decisions). It
//! interoperates with the analyzers it wraps and with GitHub code scanning.
//! `finding.v1` is deprecated and does NOT exist in this code.
//!
//! BOUNDARY RULE: does not import `keel_dsl`.

use crate::ledger::LedgerEntry;

/// Converts evidence entries into a minimal SARIF 2.1.0 log.
///
/// `properties` carries the Keel extension: this lets a downstream consumer
/// distinguish "phpstan proved it" from "a model has an opinion" without
/// losing interoperability.
pub fn to_sarif(entries: &[&LedgerEntry]) -> serde_json::Value {
    let results: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let mut result = serde_json::json!({
                "ruleId": e.rule_id,
                "level": match e.verdict {
                    keel_core::Verdict::Invalid => "error",
                    keel_core::Verdict::Unknown => "warning",
                    keel_core::Verdict::Valid => "note",
                },
                "message": { "text": e.detail.clone().unwrap_or_else(|| format!("{:?}", e.verdict)) },
                "properties": {
                    "keel": {
                        "evidenceId": e.id,
                        "origin": e.origin,
                        "snapshot": e.snapshot_hash,
                        "declaredDecision": e.declared_decision,
                        "effectiveDecision": e.effective_decision,
                        "latencyMs": e.latency_ms,
                        "tokens": e.tokens,
                    }
                }
            });
            if let Some(file) = &e.file {
                result["locations"] = serde_json::json!([{
                    "physicalLocation": {
                        "artifactLocation": { "uri": file },
                        "region": { "startLine": e.line.unwrap_or(1) }
                    }
                }]);
            }
            result
        })
        .collect();

    serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "keel",
                "version": env!("CARGO_PKG_VERSION"),
                "organization": "JhonaCodes"
            } },
            "results": results
        }]
    })
}

#[cfg(test)]
#[path = "../tests-unit/sarif.rs"]
mod tests;
