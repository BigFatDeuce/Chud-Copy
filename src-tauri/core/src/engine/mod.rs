//! Frontière entre Chud Copy et le moteur de copie.
//!
//! Le reste de l'application ne connaît que ce contrat. Remplacer rclone revient à fournir
//! une autre implémentation de [`Engine`], sans toucher à l'interface, à la validation
//! ni à l'historique.

pub mod rclone;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::errors::FriendlyError;
use crate::model::{Filters, Mode, Options};
use crate::Result;

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

/// Consigne partagée entre l'interface et les opérations en cours.
#[derive(Clone, Default)]
pub struct Control(Arc<AtomicU8>);

impl Control {
    pub fn pause(&self) {
        let _ = self.0.compare_exchange(RUNNING, PAUSED, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        let _ = self.0.compare_exchange(PAUSED, RUNNING, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn cancel(&self) {
        self.0.store(CANCELLED, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::SeqCst) == PAUSED
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CANCELLED
    }
}

/// Une liaison prête à être confiée au moteur.
#[derive(Debug, Clone)]
pub struct JobSpec {
    pub id: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub mode: Mode,
    pub options: Options,
    pub filters: Filters,
    /// Nombre de suppressions autorisées par l'analyse validée.
    pub max_delete: Option<u64>,
    /// Nom du sous-dossier d'archive de cette exécution.
    pub archive_stamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Copy,
    Update,
    Same,
    Ignored,
    Delete,
    /// Présent seulement en destination et conservé.
    Extra,
    Error,
    Conflict,
    /// Reste d'une copie interrompue.
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub rel: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProgress {
    pub name: String,
    pub size: u64,
    pub bytes: u64,
    pub percentage: u32,
    pub speed: f64,
}

/// Statistiques telles que rapportées par le moteur, jamais estimées par l'interface.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStats {
    pub bytes: u64,
    pub total_bytes: u64,
    pub speed: f64,
    pub eta: Option<f64>,
    pub transfers: u64,
    pub total_transfers: u64,
    pub checks: u64,
    pub total_checks: u64,
    pub deletes: u64,
    pub errors: u64,
    pub listed: u64,
    pub elapsed: f64,
    pub transfer_time: f64,
    pub transferring: Vec<FileProgress>,
    /// Fichiers en cours de comparaison.
    pub checking: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Notice,
    Error,
}

pub enum EngineEvent {
    Stats(EngineStats),
    Log { level: LogLevel, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub status: JobStatus,
    pub stats: EngineStats,
    pub errors: Vec<FriendlyError>,
    /// Chemins relatifs réellement écrits, pour le contrôle après transfert.
    pub written: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub entries: Vec<PlanEntry>,
    pub errors: Vec<FriendlyError>,
    pub cancelled: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyOutcome {
    pub checked: u64,
    pub failures: Vec<FriendlyError>,
    pub cancelled: bool,
}

pub trait Engine: Send + Sync {
    fn name(&self) -> String;
    fn version(&self) -> Result<String>;

    /// Calcule ce que ferait le transfert, sans rien écrire.
    fn analyze(&self, job: &JobSpec, control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> Result<Analysis>;

    fn transfer(&self, job: &JobSpec, control: &Control, events: &mut dyn FnMut(EngineEvent)) -> Result<TransferOutcome>;

    /// Relit source et destination pour confirmer les fichiers écrits.
    fn verify(&self, job: &JobSpec, files: &[String], control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> Result<VerifyOutcome>;
}
