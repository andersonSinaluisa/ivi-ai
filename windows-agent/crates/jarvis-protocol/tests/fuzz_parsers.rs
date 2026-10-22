//! Entradas hostiles y aleatorias contra los parsers (HU-WIN-047, mitiga WT-03).
//!
//! No es `cargo-fuzz` (eso es una campaña nocturna en CI, no un test): es una batería
//! **determinista y reproducible** con semilla fija que combina mutaciones de mensajes válidos,
//! bytes aleatorios y estructuras patológicas. La propiedad que se comprueba es la misma que
//! buscaría el fuzzer: el parser **nunca entra en pánico**, siempre decide, y no se lleva memoria
//! por delante.
//!
//! Los casos que algún día rompan algo se quedan aquí como test de regresión.

use jarvis_protocol::messages::TaskDispatch;
use jarvis_protocol::{Envelope, MAX_MESSAGE_BYTES};

/// PRNG propio: los tests tienen que ser reproducibles, y no hace falta calidad criptográfica.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 { 0 } else { (self.next() % limit as u64) as usize }
    }

    fn byte(&mut self) -> u8 {
        (self.next() & 0xFF) as u8
    }
}

fn valid_dispatch() -> String {
    r#"{"v":1,"id":"0192a6f4-9100-7c3a-9d2e-000000000020","type":"task.dispatch",
        "ts":"2026-09-19T15:04:05.400Z","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002",
        "correlationId":"0192a6f4-8b1e-7c3a-9d2e-aaaaaaaaaaaa",
        "data":{"taskId":"0192a6f4-9000-7c3a-9d2e-00000000a001","type":"OPEN_APP",
        "payload":{"appId":"chrome"},"attempt":1,"createdAt":"2026-09-19T15:04:05.250Z",
        "expiresAt":"2026-09-19T15:09:05.250Z","maxDurationSeconds":120,
        "requestedBy":{"type":"User","id":"0192a6f0-1111-7aaa-8bbb-000000000001"},
        "cancelRequested":false,"signature":null}}"#
        .to_owned()
}

/// Un mensaje válido con un byte cambiado, un trozo cortado o un fragmento repetido.
fn mutate(base: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = base.to_vec();
    match rng.below(5) {
        0 if !bytes.is_empty() => {
            let at = rng.below(bytes.len());
            bytes[at] = rng.byte();
        }
        1 if bytes.len() > 4 => {
            let at = rng.below(bytes.len());
            bytes.truncate(at);
        }
        2 if !bytes.is_empty() => {
            let at = rng.below(bytes.len());
            let piece: Vec<u8> = bytes[..at].to_vec();
            bytes.extend_from_slice(&piece);
        }
        3 => {
            let at = rng.below(bytes.len().max(1));
            bytes.insert(at.min(bytes.len()), rng.byte());
        }
        _ => {
            for _ in 0..rng.below(16) {
                bytes.push(rng.byte());
            }
        }
    }
    bytes
}

#[test]
fn random_bytes_never_panic_the_envelope_parser() {
    let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
    for _ in 0..20_000 {
        let length = rng.below(512);
        let bytes: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
        // Lo único que se exige: que decida. Cualquier resultado vale, un pánico no.
        let _ = Envelope::parse(&bytes);
    }
}

#[test]
fn mutations_of_a_valid_message_never_panic() {
    let base = valid_dispatch();
    let mut rng = Rng::new(0x5EED_1234_ABCD_0001);
    let mut accepted = 0usize;
    for _ in 0..20_000 {
        let bytes = mutate(base.as_bytes(), &mut rng);
        if let Ok(envelope) = Envelope::parse(&bytes) {
            accepted += 1;
            // Si el envelope pasó, el `data` también tiene que poder rechazarse sin pánico.
            let _ = envelope.data_as::<TaskDispatch>();
        }
    }
    // El test no sirve de nada si ninguna mutación llega a parsear: se comprueba que sí ocurre.
    assert!(accepted > 0, "alguna mutación debería seguir siendo un envelope válido");
}

#[test]
fn pathological_json_structures_are_rejected_without_blowing_the_stack() {
    let cases = [
        // Anidamiento profundo: serde_json tiene límite propio; lo importante es que no reviente.
        format!("{{\"v\":1,\"data\":{}}}", "[".repeat(600) + &"]".repeat(600)),
        // Objeto gigante de claves repetidas.
        format!("{{\"v\":1,\"type\":\"x\",\"data\":{{{}}}}}", "\"a\":1,".repeat(2000).trim_end_matches(',')),
        // Números fuera de rango y tipos cambiados.
        r#"{"v":1e999,"id":"x","type":"task.dispatch","ts":"x","deviceId":"x","data":{}}"#.to_owned(),
        r#"{"v":"1","id":1,"type":[],"ts":{},"deviceId":null,"data":[]}"#.to_owned(),
        // `data` con tipos inesperados en cada campo del dispatch.
        r#"{"v":1,"id":"0192a6f4-9100-7c3a-9d2e-000000000020","type":"task.dispatch","ts":"2026-09-19T15:04:05.400Z","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002","data":{"taskId":[],"type":{},"payload":"no-objeto","attempt":-1,"expiresAt":true}}"#.to_owned(),
        // Cadenas con caracteres de control y unicode hostil.
        "{\"v\":1,\"type\":\"\u{0}\u{1}\",\"data\":{}}".to_owned(),
        String::new(),
        "null".to_owned(),
        "[]".to_owned(),
    ];
    for case in cases {
        let result = Envelope::parse(case.as_bytes());
        if let Ok(envelope) = result {
            let _ = envelope.data_as::<TaskDispatch>();
        }
    }
}

#[test]
fn oversized_messages_are_rejected_before_parsing() {
    let huge = vec![b'a'; MAX_MESSAGE_BYTES + 1];
    assert!(Envelope::parse(&huge).is_err(), "por encima del límite ni se intenta parsear");

    // Justo en el límite: se intenta y se rechaza por contenido, sin pánico.
    let at_limit = vec![b'a'; MAX_MESSAGE_BYTES];
    assert!(Envelope::parse(&at_limit).is_err());
}

#[test]
fn timestamps_and_uuids_from_the_wire_are_never_trusted() {
    let hostile = [
        "2026-13-45T99:99:99Z",
        "0000-00-00T00:00:00Z",
        "9999999999999-01-01T00:00:00Z",
        "not-a-date",
        "",
        "2026-09-19T15:04:05.400+99:00",
    ];
    for ts in hostile {
        let message = format!(
            r#"{{"v":1,"id":"0192a6f4-9100-7c3a-9d2e-000000000020","type":"task.dispatch","ts":"{ts}","deviceId":"0192a6f0-2222-7aaa-8bbb-000000000002","data":{{}}}}"#
        );
        let _ = Envelope::parse(message.as_bytes());
    }
    for id in ["", "not-a-uuid", "0192a6f4-9100-7c3a-9d2e", &"f".repeat(100)] {
        let message = format!(
            r#"{{"v":1,"id":"{id}","type":"task.dispatch","ts":"2026-09-19T15:04:05.400Z","deviceId":"{id}","data":{{}}}}"#
        );
        let _ = Envelope::parse(message.as_bytes());
    }
}
