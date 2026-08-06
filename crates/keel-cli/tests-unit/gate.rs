// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `gate` (relocated out of src; included via #[path] in src/gate.rs).

    use super::*;

    #[test]
    fn claude_pretooluse_bash_maps_to_command_requested() {
        let payload = r#"{
            "session_id": "s1",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "psql -c \"DROP DATABASE prod\"" }
        }"#;
        let ev = parse_claude_code_hook(payload).unwrap();
        assert_eq!(ev.kind, EventKind::CommandRequested);
        assert_eq!(ev.command.as_deref(), Some("psql -c \"DROP DATABASE prod\""));
        assert_eq!(ev.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn claude_edit_maps_to_file_edited_with_new_content() {
        let payload = r#"{
            "session_id": "s1",
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": {
                "file_path": "lib/a.dart",
                "old_string": "x",
                "new_string": "final v = s.notifier.data;"
            }
        }"#;
        let ev = parse_claude_code_hook(payload).unwrap();
        assert_eq!(ev.kind, EventKind::FileEdited);
        assert_eq!(ev.file.as_deref(), Some("lib/a.dart"));
        assert!(ev.content.unwrap().contains(".notifier.data"));
    }

    #[test]
    fn claude_stop_maps_to_completion_requested() {
        let payload = r#"{ "session_id": "s1", "hook_event_name": "Stop" }"#;
        let ev = parse_claude_code_hook(payload).unwrap();
        assert_eq!(ev.kind, EventKind::CompletionRequested);
    }

    #[test]
    fn unknown_hooks_pass_through() {
        assert!(parse_claude_code_hook(r#"{ "hook_event_name": "Notification" }"#).is_none());
        assert!(parse_claude_code_hook("not json").is_none());
    }
