//! Tools de UI Automation de extremo a extremo (HU-WIN-065, HU-WIN-066).
//!
//! El árbol de UI lo simula el doble de plataforma; lo que se comprueba aquí es el contrato: qué
//! selectores se aceptan, qué pasa cuando hay cero o varias coincidencias, que se actúa por patrón
//! y nunca por coordenadas, y que nada se da por bueno sin un cambio observable.

mod common;

use common::Harness;
use jarvis_policy::Policy;
use jarvis_tooling::platform::UiElement;
use serde_json::{Value, json};
use uuid::Uuid;

const OWNER: &str = "0192a6f0-1111-7aaa-8bbb-000000000001";

const UI_TOOLS: [&str; 6] = ["is_app_running", "find_window", "find_element", "get_text", "invoke_element", "set_text"];

fn ui_policy() -> Policy {
    let text = format!(
        r#"
version = 1
[risk]
auto_approve_max = "R2"
approval_required_max = "R3"

[[principals]]
id = "cloud:user:{OWNER}"
capabilities = ["windows.apps.read", "windows.ui.read", "windows.ui.act"]

[privacy]
title_mode = "redact_patterns"
"#
    );
    Policy::parse(&text).expect("valid ui policy")
}

fn element(automation_id: &str, name: &str, control_type: &str, patterns: &[&str]) -> UiElement {
    UiElement {
        automation_id: automation_id.to_owned(),
        name: name.to_owned(),
        class_name: "Fake".to_owned(),
        control_type: control_type.to_owned(),
        bounds: (10, 20, 100, 40),
        enabled: true,
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        value: None,
        toggled: None,
        selected: None,
    }
}

/// Harness con una app en ejecución, su ventana y un árbol de UI mínimo.
fn harness() -> Harness {
    let harness = Harness::with_policy_and_tools(ui_policy(), &UI_TOOLS);
    harness.windows.add_running(4242, "notepad.exe", true);
    harness.windows.set_ui_title("Notas - Bloc de notas");
    harness.windows.add_ui_element(element("SaveButton", "Guardar", "Button", &["Invoke"]));
    harness.windows.add_ui_element(element("CancelButton", "Cancelar", "Button", &["Invoke"]));
    let mut editor = element("Editor", "Documento", "Document", &["Value"]);
    editor.value = Some("texto inicial".to_owned());
    harness.windows.add_ui_element(editor);
    // Dos elementos con el mismo nombre: sirven para comprobar la ambigüedad.
    harness.windows.add_ui_element(element("Item1", "Elemento", "ListItem", &["SelectionItem"]));
    harness.windows.add_ui_element(element("Item2", "Elemento", "ListItem", &["SelectionItem"]));
    harness
}

async fn run(harness: &Harness, task_type: &str, payload: Value) -> (String, Value) {
    let task_id = Uuid::now_v7();
    harness.publisher.clear();
    harness.dispatch(task_id, task_type, payload, &format!("cloud:user:{OWNER}")).await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        for kind in ["task.completed", "task.failed", "task.rejected"] {
            if harness.publisher.count(kind) > 0 {
                let data = harness.publisher.last(kind).map_or(Value::Null, |envelope| envelope.data);
                return (kind.to_owned(), data);
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "{task_type} no terminó");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

// ---- selectores (HU-WIN-065) -------------------------------------------------------

#[tokio::test]
async fn finds_an_element_by_its_automation_id() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "FIND_ELEMENT", json!({"appId": "notepad", "selector": {"automationId": "SaveButton"}})).await;
    assert_eq!(kind, "task.completed", "{data}");

    let element = &data["result"]["output"]["element"];
    assert_eq!(element["automationId"], json!("SaveButton"));
    assert_eq!(element["controlType"], json!("Button"));
    assert_eq!(element["patterns"], json!(["Invoke"]));
}

#[tokio::test]
async fn an_ambiguous_selector_is_an_error_not_a_guess() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "FIND_ELEMENT", json!({"appId": "notepad", "selector": {"name": "Elemento"}})).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("ELEMENT_AMBIGUOUS"), "{data}");
}

#[tokio::test]
async fn a_selector_that_matches_nothing_reports_element_not_found() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "FIND_ELEMENT", json!({"appId": "notepad", "selector": {"automationId": "NoExiste"}})).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("ELEMENT_NOT_FOUND"), "{data}");
    assert_eq!(data["error"]["retryable"], json!(true), "la UI puede tardar en aparecer");
}

#[tokio::test]
async fn selectors_never_accept_coordinates() {
    let harness = harness();
    for selector in [json!({"x": 100, "y": 200}), json!({"automationId": "SaveButton", "x": 10}), json!({})] {
        let (kind, data) = run(&harness, "FIND_ELEMENT", json!({"appId": "notepad", "selector": selector})).await;
        assert_eq!(kind, "task.rejected", "{selector} → {data}");
    }
}

#[tokio::test]
async fn find_window_returns_the_title_under_the_privacy_policy() {
    let harness = harness();
    harness.windows.set_ui_title("Cuenta 4111111111111111 - Banco");
    let (kind, data) = run(&harness, "FIND_WINDOW", json!({"appId": "notepad"})).await;
    assert_eq!(kind, "task.completed", "{data}");

    let output = &data["result"]["output"];
    assert_eq!(output["found"], json!(true));
    assert_eq!(output["pid"], json!(4242));
    assert_eq!(output["title"], json!("Cuenta [REDACTED] - Banco"), "el número largo se redacta");
    assert_eq!(output["titleRedacted"], json!(true));
}

#[tokio::test]
async fn automating_an_app_that_is_not_running_fails() {
    let harness = Harness::with_policy_and_tools(ui_policy(), &UI_TOOLS);
    let (kind, data) = run(&harness, "FIND_WINDOW", json!({"appId": "notepad"})).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("APP_NOT_FOUND"), "{data}");
}

#[tokio::test]
async fn an_elevated_application_is_not_automated() {
    let harness = Harness::with_policy_and_tools(ui_policy(), &UI_TOOLS);
    harness.windows.add_elevated(777, "notepad.exe");
    let (kind, data) = run(&harness, "FIND_WINDOW", json!({"appId": "notepad"})).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("APP_ELEVATED"), "UIPI: {data}");
}

// ---- lectura (HU-WIN-066) ----------------------------------------------------------

#[tokio::test]
async fn get_text_reads_the_value_pattern() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "GET_TEXT", json!({"appId": "notepad", "selector": {"automationId": "Editor"}})).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["output"]["text"], json!("texto inicial"));
    assert_eq!(data["result"]["output"]["source"], json!("ValuePattern"));
}

#[tokio::test]
async fn get_text_falls_back_to_the_accessible_name() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "GET_TEXT", json!({"appId": "notepad", "selector": {"automationId": "SaveButton"}})).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["output"]["text"], json!("Guardar"));
    assert_eq!(data["result"]["output"]["source"], json!("Name"));
}

// ---- acción (HU-WIN-066) -----------------------------------------------------------

#[tokio::test]
async fn invoking_an_element_uses_its_pattern_and_needs_an_observable_change() {
    let harness = harness();
    // El botón desaparece al pulsarse (como un diálogo que se cierra): cambio observable.
    let windows = harness.windows.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        windows.remove_ui_element("SaveButton");
    });

    let (kind, data) =
        run(&harness, "INVOKE_ELEMENT", json!({"appId": "notepad", "selector": {"automationId": "SaveButton"}})).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["output"]["pattern"], json!("Invoke"));
    assert_eq!(data["result"]["verification"]["status"], json!("PASSED"));
    assert_eq!(harness.windows.ui_invocations().len(), 1, "se actuó una sola vez");
}

#[tokio::test]
async fn an_action_without_observable_effect_fails_verification() {
    let harness = harness();
    // Nada cambia tras la acción: no se da por buena.
    let (kind, data) =
        run(&harness, "INVOKE_ELEMENT", json!({"appId": "notepad", "selector": {"automationId": "CancelButton"}}))
            .await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("VERIFICATION_FAILED"), "{data}");
}

#[tokio::test]
async fn an_element_without_action_pattern_is_never_clicked_by_coordinates() {
    let harness = harness();
    harness.windows.set_ui_invoke_pattern(None);
    let (kind, data) =
        run(&harness, "INVOKE_ELEMENT", json!({"appId": "notepad", "selector": {"automationId": "Editor"}})).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert!(format!("{data}").contains("pattern"), "{data}");
    assert!(harness.windows.ui_invocations().is_empty());
}

#[tokio::test]
async fn a_destructive_action_requires_approval() {
    let harness = harness();
    harness.windows.add_ui_element(element("DeleteButton", "Eliminar cuenta", "Button", &["Invoke"]));

    let task = Uuid::now_v7();
    harness
        .dispatch(
            task,
            "INVOKE_ELEMENT",
            json!({"appId": "notepad", "selector": {"automationId": "DeleteButton"}}),
            &format!("cloud:user:{OWNER}"),
        )
        .await;

    let required = harness.wait_for("task.approval_required", std::time::Duration::from_secs(5)).await;
    assert!(required.data["reason"].as_str().unwrap_or_default().contains("R3"), "{:?}", required.data);
    assert!(harness.windows.ui_invocations().is_empty(), "no se actúa antes de aprobar");
}

#[tokio::test]
async fn set_text_writes_through_the_value_pattern_and_verifies_it() {
    let harness = harness();
    let (kind, data) = run(
        &harness,
        "SET_TEXT",
        json!({"appId": "notepad", "selector": {"automationId": "Editor"}, "text": "hola desde jarvis"}),
    )
    .await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["verification"]["status"], json!("PASSED"));
    assert_eq!(data["result"]["verification"]["checks"][0]["name"], json!("value_equals"));
    // El texto escrito no viaja dentro del check: solo su longitud.
    assert_eq!(data["result"]["verification"]["checks"][0]["observed"]["length"], json!(17));
}

#[tokio::test]
async fn set_text_on_an_element_that_does_not_accept_text_fails() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "SET_TEXT", json!({"appId": "notepad", "selector": {"automationId": "SaveButton"}, "text": "x"}))
            .await;
    assert_eq!(kind, "task.failed", "{data}");
    assert!(format!("{data}").contains("does not accept text"), "{data}");
}

#[tokio::test]
async fn ui_tools_are_unavailable_unless_the_owner_enables_them() {
    let harness = Harness::with_policy(ui_policy());
    for task_type in ["FIND_WINDOW", "FIND_ELEMENT", "GET_TEXT", "INVOKE_ELEMENT", "SET_TEXT"] {
        let (kind, data) =
            run(&harness, task_type, json!({"appId": "notepad", "selector": {"automationId": "X"}, "text": "y"})).await;
        assert_eq!(kind, "task.rejected", "{task_type} → {data}");
        assert_eq!(data["error"]["code"], json!("UNSUPPORTED_TASK_TYPE"), "{task_type} → {data}");
    }
}
