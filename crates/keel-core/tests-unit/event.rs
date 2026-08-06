// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `event` (relocated out of src; included via #[path] in src/event.rs).

    use super::*;

    #[test]
    fn event_kind_uses_dotted_names() {
        assert_eq!(
            serde_json::to_string(&EventKind::CommandRequested).unwrap(),
            "\"command.requested\""
        );
    }

    #[test]
    fn inner_ring_is_exactly_the_spec_set() {
        let inner: Vec<EventKind> = [
            EventKind::CommandRequested,
            EventKind::TransitionRequested,
            EventKind::DeliveryRequested,
        ]
        .into();
        for k in inner {
            assert!(k.is_inner_ring());
        }
        assert!(!EventKind::FileEdited.is_inner_ring());
    }

    #[test]
    fn minimal_event_parses_from_jsonl_line() {
        let line = r#"{"kind":"file.edited","file":"lib/a.dart","content":"x.notifier.data"}"#;
        let ev: Event = serde_json::from_str(line).unwrap();
        assert_eq!(ev.kind, EventKind::FileEdited);
        assert!(ev.env.is_empty());
    }
