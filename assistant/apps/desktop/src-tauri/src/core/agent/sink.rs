use crate::domain::AgentEvent;

/// Destino de los eventos de una ejecución (la UI por un canal de Tauri, o un vector en pruebas).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

pub struct FnSink<F: Fn(AgentEvent) + Send + Sync>(pub F);

impl<F: Fn(AgentEvent) + Send + Sync> EventSink for FnSink<F> {
    fn emit(&self, event: AgentEvent) {
        (self.0)(event);
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct VecSink(pub std::sync::Mutex<Vec<AgentEvent>>);

#[cfg(test)]
impl EventSink for VecSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().expect("lock").push(event);
    }
}

#[cfg(test)]
impl VecSink {
    pub fn events(&self) -> Vec<AgentEvent> {
        self.0.lock().expect("lock").clone()
    }
}
