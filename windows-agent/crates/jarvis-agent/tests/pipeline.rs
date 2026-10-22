//! Pipeline de extremo a extremo con dobles de plataforma y transporte.
//!
//! Cubre los escenarios obligatorios de docs/11-testing/integration-scenarios.md que no requieren
//! un escritorio real: tarea válida, inválida, duplicada, permiso denegado, tool inexistente,
//! ejecución fallida, verificación fallida, desconexión, reinicio, cancelación, timeout y aprobación.

mod common;

use std::time::Duration;

use common::{ESP32, Harness, LaunchBehaviour, OWNER};
use jarvis_protocol::Channel;
use jarvis_state::LocalState;
use jarvis_tooling::platform::SessionState;
use serde_json::json;
use uuid::Uuid;

const T: Duration = Duration::from_secs(5);

/// WIN-E2E-01: tarea válida → ejecución + verificación observable → `task.completed`.
#[tokio::test]
async fn valid_task_is_completed_only_after_verification_passes() {
    let h = Harness::new();
    let task = Uuid::now_v7();
    h.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let completed = h.wait_for("task.completed", T).await;
    let result = &completed.data["result"];
    assert_eq!(result["outcome"], "SUCCEEDED");
    assert_eq!(result["verification"]["status"], "PASSED");
    assert_eq!(result["verification"]["method"], "process_and_window");
    assert_eq!(result["execution"]["status"], "SUCCEEDED");
    assert!(result["executionId"].is_string());
    assert_eq!(result["tool"], "open_app");
    assert!(h.publisher.kinds().contains(&"task.ack".to_owned()));
    assert!(h.publisher.kinds().contains(&"task.started".to_owned()));
    assert_eq!(h.agent.store.get_task(task).expect("q").expect("row").state, LocalState::Succeeded);
}

/// WIN-E2E-03: el mismo `taskId` nunca se ejecuta dos veces; se reenvía el último estado.
#[tokio::test]
async fn duplicate_dispatch_never_executes_twice() {
    let h = Harness::new();
    let task = Uuid::now_v7();
    let envelope =
        h.dispatch_envelope(task, "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}"), 300, None);

    h.agent.handle_envelope(Channel::Tasks, envelope.clone()).await;
    h.wait_for("task.completed", T).await;

    for _ in 0..10 {
        h.agent.handle_envelope(Channel::Tasks, envelope.clone()).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(h.publisher.count("task.started"), 1, "una sola ejecución");
    assert_eq!(h.windows.close_requests(), 0);
    let execution = h.agent.store.execution(task).expect("q").expect("row");
    assert_eq!(execution.status, "SUCCEEDED");
    assert!(h.publisher.count("task.completed") >= 2, "se reenvía el resultado previo");
}

/// WIN-E2E-02: payloads inválidos y aplicaciones fuera del catálogo.
#[tokio::test]
async fn invalid_payloads_are_rejected_without_executing() {
    let h = Harness::new();
    for payload in [json!({"appId": "../../cmd"}), json!({"app": "notepad"}), json!({"appId": "no-existe"})] {
        h.publisher.clear();
        h.dispatch(Uuid::now_v7(), "OPEN_APP", payload.clone(), &format!("cloud:user:{OWNER}")).await;
        let rejected = h.wait_for("task.rejected", T).await;
        assert_eq!(rejected.data["error"]["code"], "INVALID_PAYLOAD", "{payload}");
        assert_eq!(h.publisher.count("task.started"), 0);
    }
}

/// WIN-E2E-06: tipo desconocido → `UNSUPPORTED_TASK_TYPE`.
#[tokio::test]
async fn unknown_task_types_are_rejected() {
    let h = Harness::new();
    h.dispatch(Uuid::now_v7(), "FORMAT_DISK", json!({}), &format!("cloud:user:{OWNER}")).await;
    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "UNSUPPORTED_TASK_TYPE");
}

/// WIN-E2E-05: sin capability → `NOT_PERMITTED`, y nada se ejecuta.
#[tokio::test]
async fn missing_capability_is_denied() {
    let h = Harness::new();
    h.windows.add_running(10, "notepad.exe", true);
    h.dispatch(Uuid::now_v7(), "CLOSE_APP", json!({"appId": "notepad"}), &format!("cloud:device:{ESP32}")).await;

    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "NOT_PERMITTED");
    assert!(rejected.data["error"]["message"].as_str().expect("message").contains("windows.apps.close"));
    assert_eq!(h.windows.close_requests(), 0);
}

/// Un principal que la política no conoce no puede hacer nada (política sin `cloud:*`).
#[tokio::test]
async fn unknown_principals_are_denied() {
    let h = Harness::new();
    let stranger = format!("cloud:device:{}", Uuid::now_v7());
    h.dispatch(Uuid::now_v7(), "IS_APP_RUNNING", json!({"appId": "notepad"}), &stranger).await;
    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "NOT_PERMITTED");
}

/// WIN-E2E-08: la app se cierra al instante → `VERIFICATION_FAILED`, nunca `Completed`.
#[tokio::test]
async fn a_process_that_dies_immediately_is_a_verification_failure() {
    let h = Harness::new();
    h.windows.set_launch(LaunchBehaviour::ExitsWithCode(1));
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let failed = h.wait_for("task.failed", T).await;
    assert_eq!(failed.data["error"]["code"], "VERIFICATION_FAILED");
    assert!(failed.data["error"]["message"].as_str().expect("message").contains("exited with code 1"));
    assert_eq!(h.publisher.count("task.completed"), 0, "nunca se reporta éxito sin evidencia");
}

/// Proceso vivo pero sin ventana visible: tampoco es éxito.
#[tokio::test]
async fn a_process_without_a_window_is_not_success() {
    let h = Harness::new();
    h.windows.set_launch(LaunchBehaviour::NeverShowsWindow);
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let failed = h.wait_for("task.failed", Duration::from_secs(20)).await;
    assert_eq!(failed.data["error"]["code"], "VERIFICATION_FAILED");
}

/// WIN-E2E-07: el ejecutable no existe → `task.failed` con código específico.
#[tokio::test]
async fn a_missing_executable_fails_the_execution() {
    let h = Harness::new();
    h.windows.set_launch(LaunchBehaviour::NotFound);
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let failed = h.wait_for("task.failed", T).await;
    assert_eq!(failed.data["error"]["code"], "APP_NOT_FOUND");
}

/// Una app elevada no se cierra desde un agente de usuario estándar.
#[tokio::test]
async fn elevated_applications_report_app_elevated() {
    let h = Harness::new();
    h.windows.add_elevated(20, "notepad.exe");
    h.dispatch(Uuid::now_v7(), "CLOSE_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let failed = h.wait_for("task.failed", T).await;
    assert_eq!(failed.data["error"]["code"], "APP_ELEVATED");
    assert!(h.windows.terminated().is_empty());
}

/// `close_app` sin `force` sobre una app que ignora `WM_CLOSE`: la verificación falla, no se fuerza.
#[tokio::test]
async fn a_graceful_close_never_escalates_to_force() {
    let h = Harness::new();
    h.windows.add_running(30, "notepad.exe", true);
    h.windows.set_ignores_close(true);
    h.dispatch(
        Uuid::now_v7(),
        "CLOSE_APP",
        json!({"appId": "notepad", "gracePeriodMs": 1000}),
        &format!("cloud:user:{OWNER}"),
    )
    .await;

    let failed = h.wait_for("task.failed", Duration::from_secs(20)).await;
    assert_eq!(failed.data["error"]["code"], "VERIFICATION_FAILED");
    assert!(h.windows.terminated().is_empty(), "nunca se escala solo a terminación forzada");
}

/// WIN-E2E-13: `force` es R3 → aprobación local; al aprobar se ejecuta y se verifica.
#[tokio::test]
async fn force_close_requires_local_approval() {
    let h = Harness::new();
    h.windows.add_running(40, "notepad.exe", true);
    h.windows.set_ignores_close(true);
    let task = Uuid::now_v7();
    h.dispatch(
        task,
        "CLOSE_APP",
        json!({"appId": "notepad", "force": true, "gracePeriodMs": 1000}),
        &format!("cloud:user:{OWNER}"),
    )
    .await;

    let required = h.wait_for("task.approval_required", T).await;
    assert_eq!(required.data["taskId"], json!(task.to_string()));
    assert_eq!(h.agent.store.get_task(task).expect("q").expect("row").state, LocalState::AwaitingApproval);
    assert_eq!(h.publisher.count("task.started"), 0, "no se ejecuta sin aprobación");
    let pending = h.agent.store.pending_approvals().expect("approvals");
    assert_eq!(pending.len(), 1);

    h.agent.decide_approval(task, true, "local:owner").await.expect("approve");
    let completed = h.wait_for("task.completed", Duration::from_secs(20)).await;
    assert_eq!(completed.data["result"]["output"]["forced"], json!(true));
    assert_eq!(h.windows.terminated(), vec![40]);
}

/// Denegar una aprobación cancela la tarea sin ejecutarla.
#[tokio::test]
async fn denying_an_approval_cancels_the_task() {
    let h = Harness::new();
    h.windows.add_running(41, "notepad.exe", true);
    let task = Uuid::now_v7();
    h.dispatch(task, "CLOSE_APP", json!({"appId": "notepad", "force": true}), &format!("cloud:user:{OWNER}")).await;
    h.wait_for("task.approval_required", T).await;

    h.agent.decide_approval(task, false, "local:owner").await.expect("deny");
    let cancelled = h.wait_for("task.cancelled", T).await;
    assert_eq!(cancelled.data["reason"], "APPROVAL_DENIED");
    assert!(h.windows.terminated().is_empty());
}

/// WIN-E2E-12: el timeout efectivo respeta `maxDurationSeconds` del Cloud.
#[tokio::test]
async fn slow_verification_times_out() {
    let h = Harness::new();
    h.windows.set_launch(LaunchBehaviour::NeverShowsWindow);
    h.dispatch_with(
        Uuid::now_v7(),
        "OPEN_APP",
        json!({"appId": "notepad"}),
        &format!("cloud:user:{OWNER}"),
        300,
        Some(1),
    )
    .await;

    let failed = h.wait_for("task.failed", Duration::from_secs(10)).await;
    let code = failed.data["error"]["code"].as_str().expect("code").to_owned();
    assert!(code == "TOOL_TIMEOUT" || code == "VERIFICATION_FAILED", "{code}");
}

/// WIN-E2E-11: `task.cancel` de una tarea en ejecución.
#[tokio::test]
async fn a_running_task_can_be_cancelled() {
    let h = Harness::new();
    h.windows.add_running(50, "notepad.exe", true);
    h.windows.set_ignores_close(true);
    let task = Uuid::now_v7();
    h.dispatch(task, "CLOSE_APP", json!({"appId": "notepad", "gracePeriodMs": 20000}), &format!("cloud:user:{OWNER}"))
        .await;
    h.wait_for("task.started", T).await;

    let cancel = jarvis_protocol::Envelope::create(
        "task.cancel",
        h.device_id,
        &json!({"taskId": task, "reason": "USER_REQUEST"}),
        None,
        None,
    )
    .expect("envelope");
    h.agent.handle_envelope(Channel::Tasks, cancel).await;

    let cancelled = h.wait_for("task.cancelled", Duration::from_secs(10)).await;
    assert_eq!(cancelled.data["taskId"], json!(task.to_string()));
    assert!(h.windows.terminated().is_empty());
}

/// Cancelar una tarea desconocida deja un tombstone: un dispatch posterior no la ejecuta.
#[tokio::test]
async fn cancelling_an_unknown_task_prevents_a_later_dispatch() {
    let h = Harness::new();
    let task = Uuid::now_v7();
    let cancel = jarvis_protocol::Envelope::create(
        "task.cancel",
        h.device_id,
        &json!({"taskId": task, "reason": "USER_REQUEST"}),
        None,
        None,
    )
    .expect("envelope");
    h.agent.handle_envelope(Channel::Tasks, cancel).await;
    h.wait_for("task.cancelled", T).await;

    h.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.publisher.count("task.started"), 0);
}

/// Tarea vencida al llegar.
#[tokio::test]
async fn expired_tasks_are_rejected() {
    let h = Harness::new();
    h.dispatch_with(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}"), -10, None)
        .await;
    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "EXPIRED_ON_ARRIVAL");
}

/// WIN-E2E-09/10: sin conexión el resultado espera en el outbox y se publica al volver; el
/// `task.update.ack` lo asienta.
#[tokio::test]
async fn results_survive_a_disconnection_and_a_restart() {
    let h = Harness::new();
    h.publisher.set_online(false);
    let task = Uuid::now_v7();
    h.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    // La tarea se ejecuta aunque no haya conexión.
    let deadline = tokio::time::Instant::now() + T;
    while h.agent.store.get_task(task).expect("q").expect("row").state != LocalState::Succeeded {
        assert!(tokio::time::Instant::now() < deadline, "the task did not finish offline");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(h.publisher.sent().is_empty(), "nada salió mientras no había conexión");
    assert!(h.agent.open_task_ids().contains(&task));

    // Reinicio del agente: el resultado sigue en el outbox.
    let h = h.restart();
    h.agent.recover().await;
    h.agent.flush_outbox().await;
    let completed = h.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["verification"]["status"], "PASSED");

    let ack = jarvis_protocol::Envelope::create(
        "task.update.ack",
        h.device_id,
        &json!({"taskId": task, "status": "Completed"}),
        None,
        None,
    )
    .expect("envelope");
    h.agent.handle_envelope(Channel::Tasks, ack).await;
    assert!(h.agent.store.get_task(task).expect("q").expect("row").settled);
    assert!(h.agent.open_task_ids().is_empty());
}

/// W9: una ejecución interrumpida por un reinicio se reporta como fallida y **no** se repite.
#[tokio::test]
async fn an_interrupted_execution_is_never_repeated() {
    let h = Harness::new();
    let task = Uuid::now_v7();
    // Se simula el estado que deja un corte durante la ejecución.
    h.dispatch(task, "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;
    h.wait_for("task.completed", T).await;
    h.agent.store.set_state(task, LocalState::Running, &[]).expect("simulate crash");

    let h = h.restart();
    h.agent.recover().await;
    let failed = h.wait_for("task.failed", T).await;
    assert_eq!(failed.data["error"]["code"], "EXECUTION_INTERRUPTED");
    assert_eq!(h.publisher.count("task.started"), 0, "no se vuelve a ejecutar");
}

/// Tareas anteriores a la base de datos actual se rechazan (guardia `STATE_RESET`).
#[tokio::test]
async fn tasks_older_than_the_database_are_rejected() {
    let h = Harness::new();
    let mut envelope = h.dispatch_envelope(
        Uuid::now_v7(),
        "OPEN_APP",
        json!({"appId": "notepad"}),
        &format!("cloud:user:{OWNER}"),
        300,
        None,
    );
    envelope.data["createdAt"] = json!("2020-01-01T00:00:00.000Z");
    h.agent.handle_envelope(Channel::Tasks, envelope).await;

    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "EXPIRED_ON_ARRIVAL");
    assert!(rejected.data["error"]["message"].as_str().expect("message").contains("STATE_RESET"));
}

/// Con la sesión bloqueada, abrir una app falla de forma explícita; observar sigue funcionando.
#[tokio::test]
async fn a_locked_session_blocks_only_interactive_tools() {
    let h = Harness::new();
    h.windows.set_session(SessionState::Locked);
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;
    let failed = h.wait_for("task.failed", T).await;
    assert_eq!(failed.data["error"]["code"], "SESSION_UNAVAILABLE");

    h.publisher.clear();
    h.dispatch(Uuid::now_v7(), "IS_APP_RUNNING", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;
    let completed = h.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["outcome"], "SUCCEEDED");
}

/// Tras un crash loop el agente no acepta tareas nuevas, pero sí publica lo que ya tenía.
#[tokio::test]
async fn safe_mode_rejects_new_tasks() {
    let h = Harness::new();
    h.agent.set_safe_mode(true);
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;

    let rejected = h.wait_for("task.rejected", T).await;
    assert_eq!(rejected.data["error"]["code"], "NOT_PERMITTED");
    assert!(rejected.data["error"]["message"].as_str().expect("message").contains("safe mode"));
    assert_eq!(h.publisher.count("task.started"), 0);

    // Al levantarlo, el agente vuelve a trabajar.
    h.agent.set_safe_mode(false);
    h.dispatch(Uuid::now_v7(), "OPEN_APP", json!({"appId": "notepad"}), &format!("cloud:user:{OWNER}")).await;
    let completed = h.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["outcome"], "SUCCEEDED");
}

/// El alias `OPEN_CHROME` del Cloud ejecuta `open_app` con `appId=chrome` e ignora el `url`.
#[tokio::test]
async fn the_open_chrome_alias_maps_to_open_app() {
    let h = Harness::new();
    h.dispatch(Uuid::now_v7(), "OPEN_CHROME", json!({"url": "https://example.com"}), &format!("cloud:user:{OWNER}"))
        .await;
    let completed = h.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["tool"], "open_app");
    assert_eq!(completed.data["result"]["output"]["appId"], "chrome");
}

/// El título de la ventana activa se redacta y el resultado se borra tras el ack (dato sensible).
#[tokio::test]
async fn window_titles_are_redacted_after_settling() {
    let h = Harness::new();
    h.windows.add_running(60, "Code.exe", true);
    let task = Uuid::now_v7();
    h.dispatch(task, "GET_ACTIVE_WINDOW", json!({"includeTitle": true}), &format!("cloud:user:{OWNER}")).await;

    let completed = h.wait_for("task.completed", T).await;
    assert_eq!(completed.data["result"]["output"]["process"], "Code.exe");
    assert_eq!(completed.data["result"]["output"]["title"], "Jarvis - Visual Studio Code");

    let ack = jarvis_protocol::Envelope::create(
        "task.update.ack",
        h.device_id,
        &json!({"taskId": task, "status": "Completed"}),
        None,
        None,
    )
    .expect("envelope");
    h.agent.handle_envelope(Channel::Tasks, ack).await;
    let stored = h.agent.store.execution(task).expect("q").expect("row").result_json.expect("result");
    assert!(!stored.contains("Visual Studio Code"), "el título no se conserva tras el settle");
}

/// Leer el título exige una capability propia: sin ella la tarea se rechaza.
#[tokio::test]
async fn reading_the_title_requires_its_own_capability() {
    let mut policy = common::test_policy();
    for principal in &mut policy.principals {
        principal.capabilities.retain(|c| c != "windows.window.title");
    }
    let h = Harness::with_policy(policy);
    h.dispatch(Uuid::now_v7(), "GET_ACTIVE_WINDOW", json!({"includeTitle": true}), &format!("cloud:user:{OWNER}"))
        .await;
    let rejected = h.wait_for("task.rejected", T).await;
    assert!(rejected.data["error"]["message"].as_str().expect("message").contains("windows.window.title"));
}
