//! Tool `run_command`: ejecuta **comandos con nombre** del catálogo local (ADR-008).
//!
//! Lo que llega del Cloud es un `commandId` y unos parámetros tipados. El ejecutable, los
//! argumentos, el timeout y los códigos de salida esperados los definió el owner en `policy.toml`.
//! Ningún texto remoto se interpreta como línea de comandos.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jarvis_policy::CommandEntry;
use jarvis_policy::files::render_arguments;
use jarvis_tooling::descriptor::{CancellationMode, Sensitivity, Timeouts, VerificationMode, VerificationSpec};
use jarvis_tooling::platform::CommandSpec;
use jarvis_tooling::{
    Check, ErrorCode, ExecContext, ExecutionOutcome, ResourceLock, RiskLevel, Tool, ToolDescriptor, ToolError,
    VerificationReport,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::ToolContext;
use super::apps::descriptor;

/// Tope global del contrato, por encima del `timeout_ms` de cada comando.
const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
/// Cuántos comandos pueden estar ejecutándose a la vez (HU-WIN-058).
const MAX_CONCURRENT_COMMANDS: usize = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RunInput {
    command_id: String,
    #[serde(default)]
    params: Map<String, Value>,
}

pub struct RunCommand {
    ctx: Arc<ToolContext>,
    descriptor: ToolDescriptor,
    /// Límite global de comandos simultáneos (los locks por `commandId` no lo cubren).
    slots: Arc<tokio::sync::Semaphore>,
}

fn risk_of(entry: &CommandEntry) -> RiskLevel {
    match entry.risk.as_str() {
        "R2" => RiskLevel::R2,
        "R3" => RiskLevel::R3,
        // La política ya rechaza cualquier otro valor al cargarse.
        _ => RiskLevel::R1,
    }
}

impl RunCommand {
    #[must_use]
    pub fn new(ctx: Arc<ToolContext>) -> Self {
        Self {
            ctx,
            descriptor: descriptor(
                "run_command",
                "Runs a named command from the local catalog with typed parameters",
                // Riesgo base: cada comando declara el suyo (R1–R3) y `risk()` lo eleva.
                RiskLevel::R1,
                vec!["commands.run"],
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["commandId"],
                    "properties": {
                        "commandId": {"type": "string", "minLength": 1, "maxLength": 64},
                        "params": {"type": "object"}
                    }
                }),
                json!({"type": "object", "properties": {"commandId": {"type": "string"}, "exitCode": {"type": "integer"}, "stdout": {"type": "string"}, "stderr": {"type": "string"}, "truncated": {"type": "boolean"}}}),
                Timeouts { default: Duration::from_secs(30), max: MAX_COMMAND_TIMEOUT },
                CancellationMode::Kill,
                VerificationSpec {
                    mode: VerificationMode::Required,
                    method: "exit_code_expected",
                    timeout: Duration::from_secs(1),
                    poll_interval: Duration::from_millis(100),
                },
                // La salida puede contener datos del usuario: nunca en logs, redactada tras settled.
                Sensitivity::Output,
                false,
            ),
            slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_COMMANDS)),
        }
    }

    fn entry(&self, input: &Value) -> Result<(String, CommandEntry), ToolError> {
        let id =
            input.get("commandId").and_then(Value::as_str).ok_or_else(|| ToolError::invalid("missing commandId"))?;
        let policy = self.ctx.policy();
        let entry = policy
            .commands
            .get(id)
            .ok_or_else(|| ToolError::new(ErrorCode::CommandNotFound, "that command is not in the local catalog"))?;
        Ok((id.to_owned(), entry.clone()))
    }
}

#[async_trait]
impl Tool for RunCommand {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn validate(&self, input: &Value) -> Result<Value, ToolError> {
        let parsed: RunInput = serde_json::from_value(input.clone()).map_err(|e| ToolError::invalid(e.to_string()))?;
        let policy = self.ctx.policy();
        let entry = policy
            .commands
            .get(&parsed.command_id)
            .ok_or_else(|| ToolError::new(ErrorCode::CommandNotFound, "that command is not in the local catalog"))?;
        // Los parámetros se validan aquí: si uno no encaja con su tipo, la tarea se rechaza sin
        // haber lanzado nada.
        render_arguments(entry, &parsed.params).map_err(ToolError::invalid)?;
        Ok(json!({ "commandId": parsed.command_id, "params": parsed.params }))
    }

    /// El riesgo efectivo es el que declaró el owner para ese comando.
    fn risk(&self, input: &Value) -> RiskLevel {
        self.entry(input).map_or(self.descriptor.risk_level, |(_, entry)| risk_of(&entry))
    }

    /// Hace falta el permiso general **y** el del comando concreto.
    fn required_permissions(&self, input: &Value) -> Vec<String> {
        let mut permissions = vec!["commands.run".to_owned()];
        if let Some(id) = input.get("commandId").and_then(Value::as_str) {
            permissions.push(format!("commands.run:{id}"));
        }
        permissions
    }

    fn resources(&self, input: &Value) -> Vec<ResourceLock> {
        input
            .get("commandId")
            .and_then(Value::as_str)
            .map(|id| vec![ResourceLock::exclusive(format!("COMMAND:{id}"))])
            .unwrap_or_default()
    }

    async fn execute(&self, input: &Value, ctx: &ExecContext) -> Result<ExecutionOutcome, ToolError> {
        let (command_id, entry) = self.entry(input)?;
        let params = input.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
        let arguments = render_arguments(&entry, &params).map_err(ToolError::invalid)?;

        let executable = PathBuf::from(self.ctx.api.expand_env(&entry.executable));
        let working_directory = entry.working_directory.as_ref().map(|dir| PathBuf::from(self.ctx.api.expand_env(dir)));

        // El timeout es el del comando, acotado por el tope global y por lo que quede de deadline.
        let remaining = ctx.deadline.saturating_duration_since(tokio::time::Instant::now());
        let timeout = Duration::from_millis(entry.timeout_ms).min(MAX_COMMAND_TIMEOUT).min(remaining);
        if timeout.is_zero() {
            return Err(ToolError::new(ErrorCode::ToolTimeout, "no time left to run the command"));
        }

        let spec = CommandSpec {
            executable,
            arguments,
            working_directory,
            capture_output: entry.capture_output,
            max_output_bytes: entry.max_output_bytes,
            timeout,
        };

        // Límite global: esperar un hueco, sin pasarse del deadline ni ignorar la cancelación.
        let slots = self.slots.clone();
        let permit = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::new(ErrorCode::Cancelled, "cancelled while waiting for a slot")),
            permit = slots.acquire_owned() => permit.map_err(|_| ToolError::new(ErrorCode::SystemError, "the command scheduler is closed"))?,
            () = tokio::time::sleep_until(ctx.deadline) => {
                return Err(ToolError::new(ErrorCode::ToolTimeout, "too many commands are already running"));
            }
        };

        // El comando corre en un hilo bloqueante: la cancelacion viaja por esta bandera, que el
        // bucle de espera mira para matar el arbol de procesos (si no, cancelar no haria nada).
        let flag = Arc::new(jarvis_tooling::platform::CancelFlag::new(false));
        let watcher = {
            let (flag, cancel) = (flag.clone(), ctx.cancel.clone());
            tokio::spawn(async move {
                cancel.cancelled().await;
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            })
        };
        let signal = flag.clone();
        let output = self.ctx.blocking(move |api| api.run_command(&spec, &signal)).await.map_err(|error| {
            if error.code == ErrorCode::AppNotFound {
                ToolError::new(ErrorCode::CommandFailed, "the configured executable no longer exists")
            } else {
                error
            }
        });
        drop(permit);
        watcher.abort();
        let output = output?;

        if output.cancelled {
            return Err(ToolError::new(ErrorCode::Cancelled, "the command was stopped because the task was cancelled"));
        }
        if output.timed_out {
            return Err(ToolError::new(ErrorCode::ToolTimeout, "the command exceeded its timeout and was killed"));
        }

        let mut body = json!({
            "commandId": command_id,
            "exitCode": output.exit_code,
            "truncated": output.truncated,
        });
        if entry.capture_output
            && let Some(object) = body.as_object_mut()
        {
            // Salida sensible: viaja en el resultado, nunca a los logs.
            object.insert("stdout".to_owned(), Value::String(output.stdout));
            object.insert("stderr".to_owned(), Value::String(output.stderr));
        }

        Ok(ExecutionOutcome {
            output: body,
            details: Some(json!({ "method": "job_object", "capturedOutput": entry.capture_output })),
            evidence: json!({ "exitCode": output.exit_code, "expected": entry.expected_exit_codes }),
        })
    }

    /// `exit_code_expected`: el código de salida está entre los que el owner declaró.
    async fn verify(&self, _input: &Value, outcome: &ExecutionOutcome, _ctx: &ExecContext) -> VerificationReport {
        let Some(code) = outcome.evidence.get("exitCode").and_then(Value::as_i64) else {
            return VerificationReport::inconclusive(Vec::new(), "the command did not report an exit code");
        };
        let expected: Vec<i64> = outcome
            .evidence
            .get("expected")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let matched = expected.contains(&code);
        let checks = vec![Check::new("exit_code_expected", matched, Some(json!({ "exitCode": code })))];
        if matched {
            VerificationReport::passed(checks)
        } else {
            VerificationReport::failed(checks, format!("the command exited with an unexpected code ({code})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_policy::CommandParam;
    use std::collections::BTreeMap;

    fn entry() -> CommandEntry {
        CommandEntry {
            description: "ping".into(),
            executable: r"%SystemRoot%\System32\PING.EXE".into(),
            args: vec!["-n".into(), "1".into(), "{host}".into()],
            params: BTreeMap::from([(
                "host".to_owned(),
                CommandParam::Enum { values: vec!["127.0.0.1".into(), "1.1.1.1".into()] },
            )]),
            risk: "R2".into(),
            timeout_ms: 10_000,
            expected_exit_codes: vec![0],
            capture_output: true,
            max_output_bytes: 4096,
            working_directory: None,
        }
    }

    #[test]
    fn parameters_outside_the_declared_values_are_refused() {
        let entry = entry();
        // Intento de inyección: metacaracteres, otro host, argumentos extra.
        for hostile in ["127.0.0.1 && calc.exe", "8.8.8.8", "-t", "$(whoami)", "127.0.0.1\n1.1.1.1"] {
            let params = Map::from_iter([("host".to_owned(), Value::String(hostile.to_owned()))]);
            assert!(render_arguments(&entry, &params).is_err(), "debe rechazarse: {hostile}");
        }
        let params = Map::from_iter([("host".to_owned(), Value::String("127.0.0.1".to_owned()))]);
        assert_eq!(render_arguments(&entry, &params).expect("ok"), vec!["-n", "1", "127.0.0.1"]);
    }

    #[test]
    fn unknown_or_missing_parameters_are_refused() {
        let entry = entry();
        let extra = Map::from_iter([
            ("host".to_owned(), Value::String("127.0.0.1".to_owned())),
            ("other".to_owned(), Value::String("x".to_owned())),
        ]);
        assert!(render_arguments(&entry, &extra).is_err(), "un parámetro de más no se ignora en silencio");
        assert!(render_arguments(&entry, &Map::new()).is_err(), "falta el parámetro obligatorio");
    }

    #[test]
    fn the_declared_risk_of_the_command_is_the_effective_one() {
        let mut entry = entry();
        assert_eq!(risk_of(&entry), RiskLevel::R2);
        entry.risk = "R3".into();
        assert_eq!(risk_of(&entry), RiskLevel::R3);
        entry.risk = "R1".into();
        assert_eq!(risk_of(&entry), RiskLevel::R1);
    }
}
