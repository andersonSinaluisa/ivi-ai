//! Tools de P7 de extremo a extremo: sandbox de rutas, lectura de archivos, comandos con nombre,
//! foco y notificaciones (HU-WIN-054 … HU-WIN-060 y la sección de filesystem/comandos de
//! docs/11-testing/security-tests.md).
//!
//! Todo entra por `task.dispatch` como lo haría el Cloud: lo que se comprueba es lo que acaba
//! publicándose (`task.completed` / `task.rejected` / `task.failed`), no el interior de la tool.

mod common;

use common::{Harness, LaunchBehaviour};
use jarvis_policy::Policy;
use jarvis_tooling::platform::CommandOutput;
use serde_json::{Value, json};
use uuid::Uuid;

const OWNER: &str = "0192a6f0-1111-7aaa-8bbb-000000000001";

fn owner() -> String {
    format!("cloud:user:{OWNER}")
}

/// Política con raíces de filesystem y dos comandos con nombre.
fn p7_policy() -> Policy {
    let text = format!(
        r#"
version = 1
[[principals]]
id = "cloud:user:{OWNER}"
capabilities = [
  "windows.apps.read", "windows.apps.open", "windows.window.focus", "windows.notify",
  "filesystem.list", "filesystem.read", "commands.run", "commands.run:network.ping",
]

[filesystem]
allowed_roots = [
  {{ path = "C:\\Jarvis\\docs", list = true, read = true }},
  {{ path = "C:\\Jarvis\\downloads", list = true, read = false }},
]
max_read_bytes = 262144
max_list_entries = 500

[commands."network.ping"]
description = "Ping a un host conocido"
executable = "C:\\Windows\\System32\\PING.EXE"
args = ["-n", "1", "{{host}}"]
params.host = {{ type = "enum", values = ["127.0.0.1", "1.1.1.1"] }}
risk = "R1"
timeout_ms = 10000
expected_exit_codes = [0]
capture_output = true
max_output_bytes = 4096

[commands."notes.backup"]
description = "Respaldo de notas"
executable = "C:\\Jarvis\\tools\\backup.exe"
args = ["--target", "{{target}}"]
params.target = {{ type = "string", pattern = "^[A-Za-z0-9_-]{{1,32}}$" }}
risk = "R1"
timeout_ms = 30000
expected_exit_codes = [0]
"#
    );
    Policy::parse(&text).expect("valid p7 policy")
}

const P7_TOOLS: [&str; 10] = [
    "is_app_running",
    "list_running_apps",
    "get_active_window",
    "open_app",
    "close_app",
    "list_directory",
    "read_file",
    "run_command",
    "focus_window",
    "show_notification",
];

fn harness() -> Harness {
    let harness = Harness::with_policy_and_tools(p7_policy(), &P7_TOOLS);
    harness.windows.add_file(r"C:\Jarvis\docs\notes.txt", b"hola jarvis");
    harness.windows.add_file(r"C:\Jarvis\docs\cert.pem", b"-----BEGIN PRIVATE KEY-----");
    harness.windows.add_hidden_file(r"C:\Jarvis\docs\.secret", b"oculto");
    harness.windows.add_dir(r"C:\Jarvis\docs\subdir");
    harness.windows.add_file(r"C:\Jarvis\downloads\installer.txt", b"no se puede leer");
    harness.windows.add_file(r"C:\Windows\System32\config\SAM", b"registro");
    harness
}

/// Último mensaje publicado de un tipo, como JSON.
fn last(harness: &Harness, kind: &str) -> Value {
    harness.publisher.last(kind).map_or(Value::Null, |envelope| envelope.data)
}

/// Despacha una tarea y espera a su mensaje terminal. Devuelve `(tipo, data)`.
async fn run(harness: &Harness, task_type: &str, payload: Value) -> (String, Value) {
    let task_id = Uuid::now_v7();
    harness.publisher.clear();
    harness.dispatch(task_id, task_type, payload, &owner()).await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        for kind in ["task.completed", "task.failed", "task.rejected"] {
            if harness.publisher.count(kind) > 0 {
                return (kind.to_owned(), last(harness, kind));
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "{task_type} no terminó: {:?}", harness.publisher.kinds());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

// ---- sandbox de rutas (HU-WIN-054) -------------------------------------------------

#[tokio::test]
async fn hostile_paths_are_rejected_before_touching_the_disk() {
    let harness = harness();
    let hostile = [
        r"..\..\Windows\System32\config\SAM",
        r"C:\Jarvis\docs\..\..\Windows\System32\config\SAM",
        r"\\server\share\secrets.txt",
        r"\\?\C:\Jarvis\docs\notes.txt",
        r"\\.\PhysicalDrive0",
        r"C:\Jarvis\docs\notes.txt:hidden",
        r"C:\Jarvis\docs\NUL",
        r"C:\Jarvis\docs\CON.txt",
        "notes.txt",
        r"C:\Windows\System32\config\SAM",
        r"C:\Jarvis\docs2\notes.txt",
    ];
    for path in hostile {
        let (kind, data) = run(&harness, "LIST_DIRECTORY", json!({ "path": path })).await;
        assert_eq!(kind, "task.rejected", "debía rechazarse: {path} → {data}");
    }
}

#[tokio::test]
async fn a_junction_inside_an_allowed_root_is_not_followed() {
    let harness = harness();
    harness.windows.add_dir(r"C:\Jarvis\docs\link");
    harness.windows.add_reparse_point(r"C:\Jarvis\docs\link");

    // El junction solo se descubre al abrir: la tarea falla en ejecución, no en validación.
    let (kind, data) = run(&harness, "LIST_DIRECTORY", json!({ "path": r"C:\Jarvis\docs\link" })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("PATH_NOT_ALLOWED"), "{data}");
}

#[tokio::test]
async fn a_path_whose_real_target_is_outside_the_roots_is_refused() {
    let harness = harness();
    // La ruta pedida está dentro de la raíz, pero el handle dice que la real es otra (enlace
    // intermedio): manda la ruta real.
    harness.windows.add_file(r"C:\Jarvis\docs\passwd.txt", b"contenido");
    harness.windows.set_final_path(r"C:\Jarvis\docs\passwd.txt", r"C:\Users\victima\.ssh\known_hosts");

    let (kind, data) = run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\passwd.txt" })).await;
    assert_eq!(kind, "task.failed", "solo se sabe tras abrir el handle: {data}");
    assert_eq!(data["error"]["code"], json!("PATH_NOT_ALLOWED"), "{data}");
}

#[tokio::test]
async fn without_configured_roots_nothing_can_be_read() {
    let harness = Harness::with_policy_and_tools(common::test_policy(), &P7_TOOLS);
    harness.windows.add_file(r"C:\Jarvis\docs\notes.txt", b"hola");
    // La política de pruebas no tiene `[filesystem]`: ninguna raíz permitida.
    let (kind, _) = run(&harness, "LIST_DIRECTORY", json!({ "path": r"C:\Jarvis\docs" })).await;
    assert_eq!(kind, "task.rejected");
}

// ---- list_directory (HU-WIN-055) ---------------------------------------------------

#[tokio::test]
async fn lists_a_directory_without_hidden_files() {
    let harness = harness();
    let (kind, data) = run(&harness, "LIST_DIRECTORY", json!({ "path": r"C:\Jarvis\docs" })).await;
    assert_eq!(kind, "task.completed", "{data}");

    let entries = data["result"]["output"]["entries"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(names.contains(&"notes.txt"), "{names:?}");
    assert!(names.contains(&"subdir"), "{names:?}");
    assert!(!names.iter().any(|name| name.starts_with('.')), "los ocultos no se listan: {names:?}");
    assert_eq!(entries.iter().find(|e| e["name"] == "subdir").map(|e| e["kind"].clone()), Some(json!("dir")));
}

#[tokio::test]
async fn listing_is_allowed_where_reading_is_not() {
    let harness = harness();
    let (kind, data) = run(&harness, "LIST_DIRECTORY", json!({ "path": r"C:\Jarvis\downloads" })).await;
    assert_eq!(kind, "task.completed", "la raíz permite listar: {data}");

    let (kind, data) = run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\downloads\installer.txt" })).await;
    assert_eq!(kind, "task.rejected", "pero no leer: {data}");
}

// ---- read_file (HU-WIN-056) --------------------------------------------------------

#[tokio::test]
async fn reads_a_text_file_with_its_hash() {
    let harness = harness();
    let (kind, data) = run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\notes.txt" })).await;
    assert_eq!(kind, "task.completed", "{data}");

    let output = &data["result"]["output"];
    assert_eq!(output["content"], json!("hola jarvis"));
    assert_eq!(output["encoding"], json!("utf-8"));
    assert_eq!(output["size"], json!(11));
    assert_eq!(output["truncated"], json!(false));
    // sha256("hola jarvis")
    assert_eq!(
        output["sha256"],
        json!("4e21f5a25f25b7ae073997bd2705c3c80a4289a5dd852ed60a5160c847c8cb9d"),
        "el hash se calcula sobre el archivo completo"
    );
}

#[tokio::test]
async fn credential_files_are_never_returned() {
    let harness = harness();
    let (kind, data) = run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\cert.pem" })).await;
    assert_eq!(kind, "task.rejected", "{data}");
    assert!(!format!("{data}").contains("BEGIN PRIVATE KEY"), "el contenido no sale ni en el error");
}

#[tokio::test]
async fn binary_files_are_refused_instead_of_returned_as_text() {
    let harness = harness();
    harness.windows.add_file(r"C:\Jarvis\docs\image.dat", &[0x89, b'P', b'N', b'G', 0x00, 0x1A]);
    let (kind, data) = run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\image.dat" })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("BINARY_FILE"), "{data}");
}

#[tokio::test]
async fn long_files_are_read_in_windows_and_marked_as_truncated() {
    let harness = harness();
    let content = "a".repeat(5000);
    harness.windows.add_file(r"C:\Jarvis\docs\long.txt", content.as_bytes());

    let (kind, data) =
        run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\long.txt", "maxBytes": 1000 })).await;
    assert_eq!(kind, "task.completed", "{data}");
    let output = &data["result"]["output"];
    assert_eq!(output["content"].as_str().map(str::len), Some(1000));
    assert_eq!(output["truncated"], json!(true));

    let (kind, data) =
        run(&harness, "READ_FILE", json!({ "path": r"C:\Jarvis\docs\long.txt", "offset": 4500, "maxBytes": 1000 }))
            .await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["output"]["content"].as_str().map(str::len), Some(500));
    assert_eq!(data["result"]["output"]["truncated"], json!(false), "la última ventana no está truncada");
}

// ---- run_command (HU-WIN-057, HU-WIN-058) ------------------------------------------

#[tokio::test]
async fn runs_a_named_command_and_checks_its_exit_code() {
    let harness = harness();
    harness.windows.set_command_output(CommandOutput {
        exit_code: 0,
        stdout: "Respuesta desde 127.0.0.1".to_owned(),
        stderr: String::new(),
        truncated: false,
        timed_out: false,
        cancelled: false,
    });

    let (kind, data) =
        run(&harness, "RUN_COMMAND", json!({ "commandId": "network.ping", "params": { "host": "127.0.0.1" } })).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["output"]["exitCode"], json!(0));
    assert_eq!(data["result"]["verification"]["status"], json!("PASSED"), "{data}");

    // El ejecutable y los argumentos salen del catálogo, no del mensaje.
    let executed = harness.windows.commands();
    assert_eq!(executed.len(), 1);
    assert!(executed[0].executable.to_string_lossy().to_lowercase().ends_with("ping.exe"));
    assert_eq!(executed[0].arguments, vec!["-n", "1", "127.0.0.1"]);
}

#[tokio::test]
async fn an_unexpected_exit_code_fails_verification() {
    let harness = harness();
    harness.windows.set_command_output(CommandOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: "host inalcanzable".to_owned(),
        truncated: false,
        timed_out: false,
        cancelled: false,
    });

    let (kind, data) =
        run(&harness, "RUN_COMMAND", json!({ "commandId": "network.ping", "params": { "host": "1.1.1.1" } })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("VERIFICATION_FAILED"), "{data}");
}

#[tokio::test]
async fn command_injection_attempts_never_reach_the_system() {
    let harness = harness();
    let hostile = [
        json!({ "commandId": "network.ping", "params": { "host": "127.0.0.1 && calc.exe" } }),
        json!({ "commandId": "network.ping", "params": { "host": "8.8.8.8" } }),
        json!({ "commandId": "network.ping", "params": { "host": "$(whoami)" } }),
        json!({ "commandId": "network.ping", "params": { "host": "127.0.0.1", "extra": "-t" } }),
        json!({ "commandId": "notes.backup", "params": { "target": "../../../windows/system32" } }),
        json!({ "commandId": "notes.backup", "params": { "target": "ok; calc" } }),
        json!({ "commandId": "cmd.exe", "params": {} }),
        json!({ "commandId": "network.ping" }),
    ];
    for payload in hostile {
        let (kind, data) = run(&harness, "RUN_COMMAND", payload.clone()).await;
        assert_eq!(kind, "task.rejected", "debía rechazarse: {payload} → {data}");
    }
    assert!(harness.windows.commands().is_empty(), "no se ejecutó ningún proceso");
}

#[tokio::test]
async fn a_command_without_its_specific_capability_is_denied() {
    let harness = harness();
    // El principal tiene `commands.run` y `commands.run:network.ping`, pero no el de notes.backup.
    let (kind, data) =
        run(&harness, "RUN_COMMAND", json!({ "commandId": "notes.backup", "params": { "target": "diario" } })).await;
    assert_eq!(kind, "task.rejected", "{data}");
    assert_eq!(data["error"]["code"], json!("NOT_PERMITTED"), "{data}");
    assert!(harness.windows.commands().is_empty());
}

#[tokio::test]
async fn a_command_that_times_out_is_reported_as_timeout() {
    let harness = harness();
    harness.windows.set_command_output(CommandOutput {
        exit_code: -1,
        stdout: String::new(),
        stderr: String::new(),
        truncated: false,
        timed_out: true,
        cancelled: false,
    });
    let (kind, data) =
        run(&harness, "RUN_COMMAND", json!({ "commandId": "network.ping", "params": { "host": "1.1.1.1" } })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("TOOL_TIMEOUT"), "{data}");
}

// ---- focus_window (HU-WIN-059) -----------------------------------------------------

#[tokio::test]
async fn focusing_an_app_verifies_the_foreground() {
    let harness = harness();
    harness.windows.set_launch(LaunchBehaviour::AppearsAfterPolls(0));
    harness.windows.add_running(1234, "notepad.exe", true);

    let (kind, data) = run(&harness, "FOCUS_WINDOW", json!({ "appId": "notepad" })).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(data["result"]["verification"]["status"], json!("PASSED"));
    assert_eq!(harness.windows.focused_pid(), Some(1234));
}

#[tokio::test]
async fn when_windows_denies_the_foreground_the_task_fails_instead_of_lying() {
    let harness = harness();
    harness.windows.add_running(1234, "notepad.exe", true);
    harness.windows.add_running(4321, "chrome.exe", true);
    harness.windows.set_focus_allowed(false);

    let (kind, data) = run(&harness, "FOCUS_WINDOW", json!({ "appId": "chrome" })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("VERIFICATION_FAILED"), "{data}");
    assert!(format!("{data}").contains("foreground lock"), "el mensaje explica por qué: {data}");
}

#[tokio::test]
async fn focusing_an_app_that_is_not_running_fails() {
    let harness = harness();
    let (kind, data) = run(&harness, "FOCUS_WINDOW", json!({ "appId": "notepad" })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("APP_NOT_FOUND"), "{data}");
}

// ---- show_notification (HU-WIN-060) ------------------------------------------------

#[tokio::test]
async fn shows_a_plain_text_notification() {
    let harness = harness();
    let (kind, data) =
        run(&harness, "SHOW_NOTIFICATION", json!({ "title": "Jarvis", "body": "El respaldo terminó" })).await;
    assert_eq!(kind, "task.completed", "{data}");
    assert_eq!(harness.windows.notifications(), vec![("Jarvis".to_owned(), "El respaldo terminó".to_owned())]);
}

#[tokio::test]
async fn notification_text_is_sanitised_and_bounded() {
    let harness = harness();
    let long_body = "b".repeat(400);
    let (kind, data) =
        run(&harness, "SHOW_NOTIFICATION", json!({ "title": "Jarvis\r\nsegunda línea", "body": long_body })).await;
    assert_eq!(kind, "task.completed", "{data}");

    let shown = harness.windows.notifications();
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].0.contains('\n'), "sin saltos de línea en el título: {:?}", shown[0].0);
    assert!(shown[0].1.chars().count() <= 256, "cuerpo acotado: {}", shown[0].1.chars().count());
}

#[tokio::test]
async fn a_notification_that_cannot_be_shown_is_reported_as_failed() {
    let harness = harness();
    harness.windows.set_notifications_available(false);
    let (kind, data) = run(&harness, "SHOW_NOTIFICATION", json!({ "title": "Jarvis", "body": "hola" })).await;
    assert_eq!(kind, "task.failed", "{data}");
    assert_eq!(data["error"]["code"], json!("EXECUTION_FAILED"), "{data}");
}

// ---- tools deshabilitadas ----------------------------------------------------------

#[tokio::test]
async fn p7_tools_are_unavailable_unless_the_owner_enables_them() {
    let harness = Harness::with_policy(p7_policy());
    for task_type in ["LIST_DIRECTORY", "READ_FILE", "RUN_COMMAND", "FOCUS_WINDOW", "SHOW_NOTIFICATION"] {
        let (kind, data) = run(&harness, task_type, json!({ "path": r"C:\Jarvis\docs", "appId": "notepad" })).await;
        assert_eq!(kind, "task.rejected", "{task_type} → {data}");
        assert_eq!(data["error"]["code"], json!("UNSUPPORTED_TASK_TYPE"), "{task_type} → {data}");
    }
}
