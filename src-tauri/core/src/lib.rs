//! Cœur de Chud Copy.
//!
//! Tout ce qui touche aux données de l'utilisateur vit ici, sans dépendance à l'interface :
//! le modèle des tâches, les garde-fous, la persistance, la planification des opérations
//! et le moteur de copie. L'interface graphique n'est qu'un client de cette crate.

pub mod attrs;
pub mod cleanup;
pub mod drives;
pub mod engine;
pub mod errors;
pub mod filters;
pub mod health;
pub mod history;
pub mod integrity;
pub mod model;
pub mod network;
pub mod paths;
pub mod plan;
pub mod power;
pub mod proc;
pub mod recycle;
pub mod queue;
pub mod runner;
pub mod schedule;
pub mod store;
pub mod tools;
pub mod validate;
pub mod volumes;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;

pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(CoreError::Message(message.into()))
}

/// Un message pour l'interface : un code et ses paramètres. Tous les textes visibles vivent dans
/// les catalogues de langues de l'interface, le cœur n'écrit aucune phrase.
pub fn ui_text(code: &str, params: &[(&str, String)]) -> String {
    let params: serde_json::Map<String, serde_json::Value> = params.iter().map(|(key, value)| (key.to_string(), value.clone().into())).collect();
    serde_json::json!({ "code": code, "params": params }).to_string()
}

pub fn fail_text<T>(code: &str) -> Result<T> {
    fail(ui_text(code, &[]))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
