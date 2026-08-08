// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `tools` (relocated out of src; included via #[path] in src/tools.rs).

use super::*;
use keel_core::event::EventKind;

fn event_with(content: Option<&str>, command: Option<&str>) -> Event {
    Event {
        kind: EventKind::FileEdited,
        session_id: None,
        file: None,
        language: None,
        content: content.map(str::to_string),
        line: None,
        command: command.map(str::to_string),
        env: Default::default(),
        files: vec![],
        loaded_skills: vec![],
        recorded_evidence: vec![],
    }
}

fn ctx() -> ExecContext<'static> {
    ExecContext {
        workspace_root: Path::new("."),
    }
}

fn builtin_call(id: &str, with: serde_json::Value) -> CompiledToolCall {
    CompiledToolCall {
        using: CompiledToolRef::Builtin(id.into()),
        with: Some(with),
    }
}

#[test]
fn detector_contains_hits_and_misses() {
    let call = builtin_call(
        "text.contains",
        serde_json::json!({"value": ".notifier.data"}),
    );
    assert!(run_detector(
        &call,
        &event_with(Some("x.notifier.data"), None)
    ));
    assert!(!run_detector(&call, &event_with(Some("clean code"), None)));
}

#[test]
fn detector_command_classify_matches_families_and_globs() {
    let call = builtin_call(
        "command.classify",
        serde_json::json!({"families": ["psql", "*/artisan db:*"]}),
    );
    assert!(run_detector(
        &call,
        &event_with(None, Some("psql -c 'SELECT 1'"))
    ));
    assert!(run_detector(
        &call,
        &event_with(None, Some("php/artisan db:wipe --force"))
    ));
    assert!(!run_detector(&call, &event_with(None, Some("ls -la"))));
}

#[test]
fn validate_builtin_match_is_invalid_absent_content_is_unknown() {
    let call = builtin_call("text.regex", serde_json::json!({"pattern": "TODO"}));
    assert_eq!(
        run_validate(
            &call,
            &event_with(Some("x // TODO"), None),
            &Default::default(),
            ctx()
        )
        .verdict,
        Verdict::Invalid
    );
    assert_eq!(
        run_validate(
            &call,
            &event_with(Some("done"), None),
            &Default::default(),
            ctx()
        )
        .verdict,
        Verdict::Valid
    );
    assert_eq!(
        run_validate(&call, &event_with(None, None), &Default::default(), ctx()).verdict,
        Verdict::Unknown
    );
}

#[test]
fn precondition_env_and_flag_read_the_event_not_the_process() {
    let mut ev = event_with(None, Some("psql --allow-production-write -c 'SELECT 1'"));
    ev.env.insert("PROD_WRITE_ENABLED".into(), "1".into());

    let env_pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("env.present".into()),
        with: Some(serde_json::json!({"name": "PROD_WRITE_ENABLED"})),
        on_fail_declared: keel_core::Decision::Block,
    };
    let flag_pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("flag.present".into()),
        with: Some(serde_json::json!({"flag": "--allow-production-write"})),
        on_fail_declared: keel_core::Decision::Block,
    };
    assert!(run_precondition(&env_pre, &ev, &Default::default(), ctx()));
    assert!(run_precondition(&flag_pre, &ev, &Default::default(), ctx()));

    let bare = event_with(None, Some("psql -c 'SELECT 1'"));
    assert!(!run_precondition(
        &env_pre,
        &bare,
        &Default::default(),
        ctx()
    ));
    assert!(!run_precondition(
        &flag_pre,
        &bare,
        &Default::default(),
        ctx()
    ));
}

/// The cognitive gate: `skill.loaded` requires the named skill to be loaded
/// (present in the event, supplied by the broker from the store). This is how
/// keel FORCES a skill for a job — the command is blocked until it is loaded.
#[test]
fn precondition_skill_loaded_gates_on_the_session_state() {
    let pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("skill.loaded".into()),
        with: Some(serde_json::json!({"id": "web-ux-ui"})),
        on_fail_declared: keel_core::Decision::Block,
    };

    // Not loaded yet → the precondition fails (the rule will block).
    let mut ev = event_with(None, Some("git commit -m x"));
    assert!(!run_precondition(&pre, &ev, &Default::default(), ctx()));

    // After keel.skills.load recorded it, the session carries it → passes.
    ev.loaded_skills = vec!["web-ux-ui".into()];
    assert!(run_precondition(&pre, &ev, &Default::default(), ctx()));

    // A different loaded skill does not satisfy the gate.
    ev.loaded_skills = vec!["meeting-notes".into()];
    assert!(!run_precondition(&pre, &ev, &Default::default(), ctx()));
}

/// `evidence.recorded` is the generic counterpart of `skill.loaded` for any
/// event kind: gates on a (event_kind, verdict) pair the broker/gate already
/// found in the ledger for this session, never a live query from inside the
/// runtime.
#[test]
fn precondition_evidence_recorded_gates_on_session_history() {
    let pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("evidence.recorded".into()),
        with: Some(serde_json::json!({"event": "test.completed", "verdict": "invalid"})),
        on_fail_declared: keel_core::Decision::Block,
    };

    // No evidence yet → fails (the rule will block).
    let mut ev = event_with(None, Some("git push"));
    assert!(!run_precondition(&pre, &ev, &Default::default(), ctx()));

    // Exact (event_kind, verdict) pair recorded → passes.
    ev.recorded_evidence = vec![(EventKind::TestCompleted, Verdict::Invalid)];
    assert!(run_precondition(&pre, &ev, &Default::default(), ctx()));

    // Same event_kind, different verdict → does not satisfy the gate.
    ev.recorded_evidence = vec![(EventKind::TestCompleted, Verdict::Valid)];
    assert!(!run_precondition(&pre, &ev, &Default::default(), ctx()));

    // Omitting `verdict` in the rule matches any verdict for that event_kind.
    let any_verdict_pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("evidence.recorded".into()),
        with: Some(serde_json::json!({"event": "test.completed"})),
        on_fail_declared: keel_core::Decision::Block,
    };
    assert!(run_precondition(
        &any_verdict_pre,
        &ev,
        &Default::default(),
        ctx()
    ));

    // A malformed `event` parameter fails closed, not open.
    let malformed_pre = CompiledPrecondition {
        using: CompiledToolRef::Builtin("evidence.recorded".into()),
        with: Some(serde_json::json!({"event": "not.a.real.event"})),
        on_fail_declared: keel_core::Decision::Block,
    };
    assert!(!run_precondition(
        &malformed_pre,
        &ev,
        &Default::default(),
        ctx()
    ));
}

/// FAIL-SAFE: missing binary → `unknown`, never a crash (section 6.4).
#[test]
fn missing_binary_yields_unknown_not_crash() {
    let def = ExternalToolDef {
        id: "ghost".into(),
        command: vec!["/nonexistent/keel-ghost-tool".into()],
        timeout_ms: 1000,
        output: OutputKind::VerdictJson,
    };
    let out = run_external(&def, &event_with(None, None), ctx());
    assert_eq!(out.verdict, Verdict::Unknown);
    assert_eq!(out.origin, OriginClass::Deterministic);
}

/// FAIL-SAFE: timeout → process killed → `unknown`.
#[test]
fn timeout_yields_unknown() {
    let def = ExternalToolDef {
        id: "sleeper".into(),
        command: vec!["sleep".into(), "5".into()],
        timeout_ms: 100,
        output: OutputKind::ExitCode,
    };
    let out = run_external(&def, &event_with(None, None), ctx());
    assert_eq!(out.verdict, Verdict::Unknown);
    assert!(out.latency_ms < 3000, "the timeout must cut off early");
}

#[test]
fn exit_code_tool_maps_three_states() {
    let mk = |code: &str| ExternalToolDef {
        id: "sh".into(),
        command: vec!["sh".into(), "-c".into(), format!("exit {code}")],
        timeout_ms: 2000,
        output: OutputKind::ExitCode,
    };
    assert_eq!(
        run_external(&mk("0"), &event_with(None, None), ctx()).verdict,
        Verdict::Valid
    );
    assert_eq!(
        run_external(&mk("1"), &event_with(None, None), ctx()).verdict,
        Verdict::Invalid
    );
    assert_eq!(
        run_external(&mk("2"), &event_with(None, None), ctx()).verdict,
        Verdict::Unknown
    );
}

#[test]
fn verdict_json_tool_roundtrips_findings() {
    let def = ExternalToolDef {
            id: "echoer".into(),
            command: vec![
                "sh".into(),
                "-c".into(),
                r#"cat > /dev/null; echo '{"verdict":"invalid","findings":[{"message":"raw query","file":"a.php","line":3}]}'"#.into(),
            ],
            timeout_ms: 2000,
            output: OutputKind::VerdictJson,
        };
    let out = run_external(&def, &event_with(Some("x"), None), ctx());
    assert_eq!(out.verdict, Verdict::Invalid);
    assert_eq!(out.findings[0].file.as_deref(), Some("a.php"));
}

#[test]
fn sarif_output_empty_results_is_valid() {
    let def = ExternalToolDef {
        id: "sarif-clean".into(),
        command: vec![
            "sh".into(),
            "-c".into(),
            r#"cat > /dev/null; echo '{"version":"2.1.0","runs":[{"results":[]}]}'"#.into(),
        ],
        timeout_ms: 2000,
        output: OutputKind::Sarif,
    };
    assert_eq!(
        run_external(&def, &event_with(None, None), ctx()).verdict,
        Verdict::Valid
    );
}
