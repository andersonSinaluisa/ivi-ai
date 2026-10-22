//! Garantías del estado local: idempotencia, outbox durable, una sola ejecución y recuperación.

use chrono::{Duration, Utc};
use jarvis_state::store::FinishExecution;
use jarvis_state::{ApprovalState, LocalState, NewTask, OutboxMessage, Store};
use uuid::Uuid;

fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("state").join("agent.db");
    (dir, path)
}

fn new_task(task_id: Uuid, state: LocalState) -> NewTask {
    NewTask {
        task_id,
        correlation_id: Some(Uuid::now_v7()),
        task_type: "OPEN_APP".into(),
        tool: Some("open_app".into()),
        principal: "cloud:user:0192a6f0-0000-7aaa-8bbb-00000000000a".into(),
        payload: serde_json::json!({"appId": "notepad"}),
        state,
        risk_level: Some("R2".into()),
        decision: Some("APPROVED".into()),
        decision_reason: Some("AUTO_APPROVED".into()),
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        max_duration_seconds: Some(60),
        sensitive: false,
        signed_by: None,
    }
}

fn msg(kind: &str) -> OutboxMessage {
    OutboxMessage::new(kind, format!(r#"{{"type":"{kind}"}}"#))
}

#[test]
fn a_task_is_stored_with_its_outbox_message_atomically() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let task = Uuid::now_v7();
    store.insert_task(&new_task(task, LocalState::Accepted), &[msg("task.ack")]).expect("insert");

    let row = store.get_task(task).expect("query").expect("present");
    assert_eq!(row.state, LocalState::Accepted);
    assert!(!row.settled);
    assert_eq!(store.pending_outbox().expect("outbox").len(), 1);
    assert!(store.last_message(task).expect("last").is_some());
    assert_eq!(store.open_task_ids(256).expect("open ids"), vec![task]);
}

#[test]
fn only_one_execution_per_task_is_possible() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let task = Uuid::now_v7();
    store.insert_task(&new_task(task, LocalState::Accepted), &[]).expect("insert");

    store.start_execution(task, Uuid::now_v7(), Utc::now(), &[msg("task.started")]).expect("first");
    let second = store.start_execution(task, Uuid::now_v7(), Utc::now(), &[]);
    assert!(matches!(second, Err(jarvis_state::StoreError::AlreadyExecuted(t)) if t == task));
}

#[test]
fn results_survive_a_restart_until_the_cloud_acks_them() {
    let (_dir, path) = temp_db();
    let task = Uuid::now_v7();
    {
        let store = Store::open(&path).expect("open");
        store.insert_task(&new_task(task, LocalState::Accepted), &[msg("task.ack")]).expect("insert");
        store.start_execution(task, Uuid::now_v7(), Utc::now(), &[msg("task.started")]).expect("start");
        let finish = FinishExecution {
            status: "SUCCEEDED".into(),
            verification_status: "PASSED".into(),
            result_json: r#"{"outcome":"SUCCEEDED"}"#.into(),
            error_code: None,
        };
        store.finish_execution(task, LocalState::Succeeded, &finish, &[msg("task.completed")]).expect("finish");
        for (seq, _) in store.pending_outbox().expect("pending") {
            store.mark_published(seq).expect("published");
        }
    }

    // Reinicio del proceso: el resultado sigue ahí y se reenvía hasta recibir el ack.
    let store = Store::open(&path).expect("reopen");
    assert_eq!(store.get_task(task).expect("q").expect("row").state, LocalState::Succeeded);
    assert_eq!(store.resend_candidates().expect("resend").len(), 1);
    assert_eq!(store.open_task_ids(256).expect("ids"), vec![task]);

    store.settle(task).expect("settle");
    assert!(store.get_task(task).expect("q").expect("row").settled);
    assert!(store.resend_candidates().expect("resend").is_empty());
    assert!(store.open_task_ids(256).expect("ids").is_empty());
}

#[test]
fn sensitive_results_are_redacted_after_settling() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let task = Uuid::now_v7();
    let mut new = new_task(task, LocalState::Accepted);
    new.sensitive = true;
    store.insert_task(&new, &[]).expect("insert");
    store.start_execution(task, Uuid::now_v7(), Utc::now(), &[]).expect("start");
    let finish = FinishExecution {
        status: "SUCCEEDED".into(),
        verification_status: "NOT_APPLICABLE".into(),
        result_json: r#"{"output":{"title":"CANARY-secret"}}"#.into(),
        error_code: None,
    };
    store.finish_execution(task, LocalState::Succeeded, &finish, &[msg("task.completed")]).expect("finish");

    assert!(store.settle(task).expect("settle"), "sensitive tasks report that they need redaction");
    store.redact(task, r#"{"redacted":true}"#, "{}").expect("redact");
    let execution = store.execution(task).expect("q").expect("row");
    assert_eq!(execution.result_json.as_deref(), Some(r#"{"redacted":true}"#));
    assert!(!store.get_task(task).expect("q").expect("row").payload.to_string().contains("notepad"));
}

#[test]
fn recovery_finds_interrupted_and_queued_tasks() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let running = Uuid::now_v7();
    let queued = Uuid::now_v7();
    store.insert_task(&new_task(running, LocalState::Accepted), &[]).expect("insert");
    store.start_execution(running, Uuid::now_v7(), Utc::now(), &[]).expect("start");
    store.insert_task(&new_task(queued, LocalState::Accepted), &[]).expect("insert");

    let interrupted = store.tasks_in(&[LocalState::Running, LocalState::Verifying]).expect("interrupted");
    assert_eq!(interrupted.iter().map(|t| t.task_id).collect::<Vec<_>>(), vec![running]);
    let accepted = store.tasks_in(&[LocalState::Accepted]).expect("accepted");
    assert_eq!(accepted.iter().map(|t| t.task_id).collect::<Vec<_>>(), vec![queued]);
}

#[test]
fn approvals_are_durable_and_expire() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let task = Uuid::now_v7();
    store.insert_task(&new_task(task, LocalState::AwaitingApproval), &[]).expect("insert");
    store.create_approval(task, "sha256:abc", Utc::now() + Duration::seconds(300)).expect("create");

    let pending = store.pending_approvals().expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool.as_deref(), Some("open_app"));
    assert!(store.expired_approvals(Utc::now()).expect("expired").is_empty());
    assert_eq!(store.expired_approvals(Utc::now() + Duration::seconds(301)).expect("expired"), vec![task]);

    assert!(store.decide_approval(task, ApprovalState::Granted, "local:owner").expect("decide"));
    assert!(!store.decide_approval(task, ApprovalState::Denied, "local:owner").expect("decide"), "solo una decisión");
    assert_eq!(store.approval(task).expect("q").expect("row").state, ApprovalState::Granted);
}

#[test]
fn settled_tasks_are_deleted_after_the_retention_window() {
    let (_dir, path) = temp_db();
    let store = Store::open(&path).expect("open");
    let task = Uuid::now_v7();
    store.insert_task(&new_task(task, LocalState::Succeeded), &[msg("task.completed")]).expect("insert");
    store.settle(task).expect("settle");

    assert_eq!(store.cleanup(8, Utc::now()).expect("cleanup"), 0, "dentro de la ventana no se borra");
    assert_eq!(store.cleanup(8, Utc::now() + Duration::days(9)).expect("cleanup"), 1);
    assert!(store.get_task(task).expect("q").is_none());
}

#[test]
fn a_corrupt_database_is_set_aside_and_recreated() {
    let (_dir, path) = temp_db();
    {
        let store = Store::open(&path).expect("open");
        store.insert_task(&new_task(Uuid::now_v7(), LocalState::Accepted), &[]).expect("insert");
    }
    let mut bytes = std::fs::read(&path).expect("read");
    for byte in bytes.iter_mut().skip(1024).take(4096) {
        *byte = 0xff;
    }
    std::fs::write(&path, &bytes).expect("write");

    let (store, was_reset) = Store::open_or_reset(&path).expect("reset");
    assert!(was_reset, "una base corrupta se aparta y se crea una nueva");
    assert_eq!(store.stats().expect("stats").tasks, 0);
    let aside = std::fs::read_dir(path.parent().expect("parent"))
        .expect("dir")
        .filter_map(std::result::Result::ok)
        .any(|e| e.file_name().to_string_lossy().contains("corrupt"));
    assert!(aside, "la base corrupta se conserva para diagnóstico");
}
