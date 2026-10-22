//! Vectores de prueba compartidos de la firma end-to-end (ADR-012 del Cloud).
//!
//! El archivo `api/docs/03-protocols/signature-vectors.json` es normativo: define la
//! canonicalización del payload y las cadenas que se firman, para que cualquier cliente nuevo
//! (Owner Console, móvil, ESP32) pueda firmar sin leer este código. Esta prueba comprueba que la
//! implementación del agente sigue produciendo exactamente eso.
//!
//! Para regenerarlos tras un cambio **deliberado** del formato:
//! `JARVIS_WRITE_SIGNATURE_VECTORS=1 cargo test -p jarvis-protocol --test signature_vectors`

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use jarvis_protocol::messages::RequestedBy;
use jarvis_protocol::signature::{self, PublicKey, Signature};
use serde_json::{Value, json};
use uuid::Uuid;

fn vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../api/docs/03-protocols/signature-vectors.json")
}

/// Semilla fija: los vectores deben ser reproducibles.
const SEED: [u8; 32] = [7u8; 32];
const KEY_ID: &str = "owner-laptop-1";

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - index * 6)) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn canonical_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("objeto vacio", json!({})),
        ("claves desordenadas y anidadas", json!({"b": 1, "a": {"y": true, "x": [1, 2]}})),
        ("nulos, booleanos y negativos", json!({"n": null, "t": true, "f": false, "i": -42, "z": 0})),
        ("cadenas con escapes", json!({"nl": "a\nb", "quote": "say \"hi\"", "back": "c\\d", "ctrl": "\u{1}"})),
        // Orden por unidades UTF-16 (RFC 8785): el emoji (surrogate D83D) va ANTES que Ｚ (FF3A),
        // al reves de lo que daria ordenar por punto de codigo. Es el vector que distingue ambas reglas.
        ("orden UTF-16 con no-ASCII", json!({"a": 1, "\u{e9}": 2, "\u{ff3a}": 3, "\u{1f600}": 4})),
        ("listas de objetos", json!({"items": [{"b": 2, "a": 1}, {"d": [true, null]}]})),
        ("payload real del MVP", json!({"appId": "chrome"})),
    ]
}

fn task_case() -> (Uuid, &'static str, Value, RequestedBy, DateTime<Utc>) {
    (
        Uuid::parse_str("019263f0-0000-7000-8000-000000000001").expect("uuid"),
        "CLOSE_APP",
        json!({"appId": "notepad", "force": true}),
        RequestedBy {
            kind: "User".to_owned(),
            id: Uuid::parse_str("019263f0-0000-7000-8000-0000000000aa").expect("uuid"),
        },
        DateTime::parse_from_rfc3339("2026-09-23T10:00:00.000Z").expect("ts").with_timezone(&Utc),
    )
}

fn decision_case() -> (Uuid, &'static str, Value, &'static str, DateTime<Utc>) {
    (
        Uuid::parse_str("019263f0-0000-7000-8000-000000000001").expect("uuid"),
        "GRANTED",
        json!({"appId": "notepad", "force": true}),
        "b6d0f1a2c3e44f5a",
        DateTime::parse_from_rfc3339("2026-09-23T10:01:30.500Z").expect("ts").with_timezone(&Utc),
    )
}

fn build() -> Value {
    use ed25519_dalek::{Signer, SigningKey};
    let signing = SigningKey::from_bytes(&SEED);

    let canonical = canonical_cases()
        .into_iter()
        .map(|(name, input)| {
            json!({
                "name": name,
                "input": input,
                "canonical": signature::canonical_json(&input),
                "payloadSha256": signature::payload_hash(&input),
            })
        })
        .collect::<Vec<_>>();

    let (task_id, task_type, payload, requested_by, expires_at) = task_case();
    let task_message = signature::task_message(task_id, task_type, &payload, &requested_by, expires_at);

    let (d_task, decision, d_payload, nonce, decided_at) = decision_case();
    let decision_message =
        signature::decision_message(d_task, decision, &signature::payload_hash(&d_payload), nonce, decided_at);

    json!({
        "schema": "jarvis.signature-vectors/1",
        "note": "Normativo (ADR-012). Generado por windows-agent/crates/jarvis-protocol/tests/signature_vectors.rs.",
        "key": {
            "alg": "Ed25519",
            "keyId": KEY_ID,
            "seedBase64": encode_base64(&SEED),
            "publicKeyBase64": encode_base64(signing.verifying_key().as_bytes()),
        },
        "canonicalJson": canonical,
        "taskSignature": {
            "taskId": task_id,
            "type": task_type,
            "payload": payload,
            "requestedBy": { "type": requested_by.kind, "id": requested_by.id },
            "expiresAt": "2026-09-23T10:00:00.000Z",
            "message": task_message,
            "signature": {
                "alg": "Ed25519",
                "keyId": KEY_ID,
                "value": encode_base64(&signing.sign(task_message.as_bytes()).to_bytes()),
            },
        },
        "approvalDecisionSignature": {
            "taskId": d_task,
            "decision": decision,
            "payload": d_payload,
            "payloadSha256": signature::payload_hash(&d_payload),
            "nonce": nonce,
            "decidedAt": "2026-09-23T10:01:30.500Z",
            "message": decision_message,
            "signature": {
                "alg": "Ed25519",
                "keyId": KEY_ID,
                "value": encode_base64(&signing.sign(decision_message.as_bytes()).to_bytes()),
            },
        },
    })
}

#[test]
fn the_shared_vectors_match_this_implementation() {
    let path = vectors_path();
    let built = build();

    if std::env::var("JARVIS_WRITE_SIGNATURE_VECTORS").is_ok() {
        let text = format!("{}\n", serde_json::to_string_pretty(&built).expect("serialize"));
        std::fs::write(&path, text).expect("escribir vectores");
        eprintln!("vectores regenerados en {}", path.display());
        return;
    }

    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("SKIPPED: no hay api/docs junto a windows-agent");
        return;
    };
    let stored: Value = serde_json::from_str(&text).expect("los vectores deben ser JSON valido");

    // Comparar campo a campo da un fallo legible: dice qué vector se rompió, no "los JSON difieren".
    for (index, case) in stored["canonicalJson"].as_array().expect("canonicalJson").iter().enumerate() {
        let input = &case["input"];
        let name = case["name"].as_str().unwrap_or("?");
        assert_eq!(signature::canonical_json(input), case["canonical"], "canonical [{index}] {name}");
        assert_eq!(signature::payload_hash(input), case["payloadSha256"], "payloadSha256 [{index}] {name}");
    }

    assert_eq!(built["taskSignature"]["message"], stored["taskSignature"]["message"], "cadena firmada de tarea");
    assert_eq!(
        built["approvalDecisionSignature"]["message"], stored["approvalDecisionSignature"]["message"],
        "cadena firmada de decision"
    );
    assert_eq!(built["key"]["publicKeyBase64"], stored["key"]["publicKeyBase64"], "clave publica del vector");

    // Y la firma guardada tiene que verificar de verdad contra la clave publica guardada.
    let public = PublicKey::from_base64(KEY_ID, stored["key"]["publicKeyBase64"].as_str().expect("clave"))
        .expect("clave publica valida");
    for section in ["taskSignature", "approvalDecisionSignature"] {
        let signature: Signature =
            serde_json::from_value(stored[section]["signature"].clone()).expect("firma del vector");
        let message = stored[section]["message"].as_str().expect("mensaje");
        assert_eq!(
            signature::verify(message, &signature, std::slice::from_ref(&public)),
            Ok(KEY_ID.to_owned()),
            "la firma guardada en {section} debe verificar"
        );
    }
}
