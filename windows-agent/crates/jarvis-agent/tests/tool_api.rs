//! Local Tool API (EPIC-16): contrato JSON-RPC, política y límites.
//!
//! Lo que se comprueba es que este canal **no** es una puerta trasera: mismas reglas que una tarea
//! del Cloud, principal propio (`local:agent-core`), presupuesto de llamadas y deduplicación.

mod common;

use common::Harness;
use jarvis_agent::tool_api::{NOT_PERMITTED, RATE_LIMITED, TOOL_FAILED, ToolApi};
use jarvis_policy::Policy;
use serde_json::{Value, json};

/// Política con permisos para el Agent Core (lectura y abrir apps, nada de cerrar).
fn core_policy() -> Policy {
    Policy::parse(
        r#"
version = 1
[[principals]]
id = "local:agent-core"
capabilities = ["windows.apps.read", "windows.apps.open"]

[[principals]]
id = "local:owner"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force"]
"#,
    )
    .expect("valid policy")
}

fn call(id: i64, name: &str, arguments: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
        .to_string()
}

async fn response(api: &ToolApi, line: &str) -> Value {
    serde_json::from_str(&api.handle_line(line).await).expect("json-rpc response")
}

#[tokio::test]
async fn tools_list_describes_the_enabled_tools_for_an_llm_client() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);

    let reply = response(&api, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string()).await;
    assert_eq!(reply["jsonrpc"], json!("2.0"));
    assert_eq!(reply["result"]["principal"], json!("local:agent-core"));

    let tools = reply["result"]["tools"].as_array().cloned().unwrap_or_default();
    assert_eq!(tools.len(), 5, "las cinco del MVP");
    let open = tools.iter().find(|tool| tool["name"] == "open_app").expect("open_app");
    // Lo que un cliente necesita para llamar: esquema, riesgo y permisos.
    assert_eq!(open["inputSchema"]["required"], json!(["appId"]));
    assert_eq!(open["riskLevel"], json!("R2"));
    assert_eq!(open["requiredPermissions"], json!(["windows.apps.open"]));
    assert!(open["description"].as_str().is_some_and(|d| !d.is_empty()));
}

#[tokio::test]
async fn a_tool_call_runs_the_same_pipeline_and_returns_a_tool_result() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);

    let reply = response(&api, &call(2, "open_app", json!({"appId": "notepad"}))).await;
    let result = &reply["result"]["result"];
    assert_eq!(result["outcome"], json!("SUCCEEDED"), "{reply}");
    assert_eq!(result["verification"]["status"], json!("PASSED"));
    assert_eq!(result["schema"], json!("jarvis.tool-result/1"), "el mismo ToolResult que el Cloud");
}

#[tokio::test]
async fn the_local_api_cannot_borrow_the_owner_permissions() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);
    harness.windows.add_running(1234, "notepad.exe", true);

    // El owner sí puede cerrar apps; `local:agent-core` no, y no hay forma de pedirlo por aquí.
    let reply = response(&api, &call(3, "close_app", json!({"appId": "notepad"}))).await;
    assert_eq!(reply["error"]["code"], json!(NOT_PERMITTED), "{reply}");
    assert!(reply["error"]["message"].as_str().unwrap_or_default().contains("windows.apps.close"), "{reply}");
    assert_eq!(harness.windows.close_requests(), 0, "no se tocó la app");
}

#[tokio::test]
async fn an_unknown_agent_core_principal_is_denied_by_default() {
    // La política de pruebas no declara `local:agent-core`: sin regla, no hay permiso.
    let harness = Harness::new();
    let api = ToolApi::new(harness.agent.clone(), 60);

    let reply = response(&api, &call(4, "open_app", json!({"appId": "notepad"}))).await;
    assert_eq!(reply["error"]["code"], json!(NOT_PERMITTED), "{reply}");
}

#[tokio::test]
async fn a_failing_tool_reports_why_with_its_checks() {
    let harness = Harness::with_policy(core_policy());
    harness.windows.set_launch(common::LaunchBehaviour::NotFound);
    let api = ToolApi::new(harness.agent.clone(), 60);

    let reply = response(&api, &call(5, "open_app", json!({"appId": "notepad"}))).await;
    assert_eq!(reply["error"]["code"], json!(TOOL_FAILED), "{reply}");
    // El `ToolResult` completo viaja en `data`: el llamante puede razonar sobre el fallo.
    assert_eq!(reply["error"]["data"]["outcome"], json!("FAILED"));
    assert!(reply["error"]["data"]["error"]["code"].as_str().is_some(), "{reply}");
}

#[tokio::test]
async fn the_call_budget_stops_a_runaway_loop() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 3);

    for id in 0..3 {
        let reply = response(&api, &call(id, "is_app_running", json!({"appId": "notepad"}))).await;
        assert!(reply.get("result").is_some(), "las primeras entran: {reply}");
    }
    let reply = response(&api, &call(9, "is_app_running", json!({"appId": "notepad"}))).await;
    assert_eq!(reply["error"]["code"], json!(RATE_LIMITED), "{reply}");
}

#[tokio::test]
async fn repeating_an_invocation_id_does_not_repeat_the_effect() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);
    let line = json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": {"name": "open_app", "arguments": {"appId": "notepad"}, "invocationId": "abc-123"}
    })
    .to_string();

    let first = response(&api, &line).await;
    assert_eq!(first["result"]["result"]["outcome"], json!("SUCCEEDED"), "{first}");

    let second = response(&api, &line).await;
    assert_eq!(second["result"]["deduplicated"], json!(true), "{second}");
    assert_eq!(second["result"]["result"]["executionId"], first["result"]["result"]["executionId"]);
}

#[tokio::test]
async fn malformed_requests_get_json_rpc_errors() {
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);

    let cases = [
        ("no es json", -32700),
        (r#"{"jsonrpc":"1.0","id":1,"method":"tools/list"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":1,"method":"tools/delete"}"#, -32601),
        (r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{}}}"#, -32602),
        (r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"no_such_tool"}}"#, -32602),
    ];
    for (line, expected) in cases {
        let reply = response(&api, line).await;
        assert_eq!(reply["error"]["code"], json!(expected), "{line} → {reply}");
    }
}

#[tokio::test]
async fn tool_calls_from_the_local_api_are_never_llm_aware() {
    // El contrato no tiene hueco para prompts ni modelos: solo nombre, argumentos e id.
    let harness = Harness::with_policy(core_policy());
    let api = ToolApi::new(harness.agent.clone(), 60);
    let line = json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": "is_app_running", "arguments": {"appId": "notepad"}, "model": "claude"}
    })
    .to_string();

    let reply = response(&api, &line).await;
    assert_eq!(reply["error"]["code"], json!(-32602), "un campo desconocido se rechaza: {reply}");
}
