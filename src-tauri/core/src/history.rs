//! Historique des opérations : rapports et journaux détaillés.
//!
//! L'historique ne fait que décrire le passé. Rien ici ne relance une opération : une exécution
//! surprise par un arrêt de l'application est seulement marquée comme interrompue.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::engine::{FileProgress, JobStatus};
use crate::errors::FriendlyError;
use crate::model::Mode;
use crate::store::{atomic_write, Store};
use crate::{fail_text, ui_text, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    Paused,
    Cancelling,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunStatus {
    pub fn is_active(self) -> bool {
        matches!(self, RunStatus::Running | RunStatus::Paused | RunStatus::Cancelling)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkPhase {
    Waiting,
    Copying,
    Verifying,
    Done,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkFile {
    pub name: String,
    pub size: u64,
    pub bytes: u64,
    pub percentage: u32,
    pub speed: f64,
}

impl From<&FileProgress> for LinkFile {
    fn from(file: &FileProgress) -> Self {
        Self {
            name: file.name.clone(),
            size: file.size,
            bytes: file.bytes,
            percentage: file.percentage,
            speed: file.speed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkRecord {
    pub id: String,
    pub source: String,
    pub target: String,
    pub phase: LinkPhase,
    pub status: Option<JobStatus>,
    pub bytes: u64,
    pub total_bytes: u64,
    pub transfers: u64,
    pub total_transfers: u64,
    pub checks: u64,
    pub total_checks: u64,
    pub deletes: u64,
    pub errors: u64,
    pub speed: f64,
    pub average_speed: f64,
    pub eta: Option<f64>,
    pub elapsed: f64,
    pub verify_checked: u64,
    pub verify_total: u64,
    pub verify_failures: u64,
    pub current: Vec<LinkFile>,
    pub message: Option<String>,
}

impl Default for LinkRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            source: String::new(),
            target: String::new(),
            phase: LinkPhase::Waiting,
            status: None,
            bytes: 0,
            total_bytes: 0,
            transfers: 0,
            total_transfers: 0,
            checks: 0,
            total_checks: 0,
            deletes: 0,
            errors: 0,
            speed: 0.0,
            average_speed: 0.0,
            eta: None,
            elapsed: 0.0,
            verify_checked: 0,
            verify_total: 0,
            verify_failures: 0,
            current: Vec::new(),
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Totals {
    pub bytes: u64,
    pub total_bytes: u64,
    pub transfers: u64,
    pub total_transfers: u64,
    pub deletes: u64,
    pub errors: u64,
    pub speed: f64,
    pub average_speed: f64,
    pub eta: Option<f64>,
    /// Faux tant qu'une liaison n'a pas fini d'établir son total réel.
    pub total_known: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct RunError {
    pub link_id: String,
    pub target: String,
    #[serde(flatten)]
    pub error: Option<FriendlyError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: Mode,
    pub status: RunStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub elapsed: f64,
    pub links: Vec<LinkRecord>,
    pub totals: Totals,
    pub errors: Vec<RunError>,
    pub errors_total: u64,
}

impl Default for RunRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            task_id: String::new(),
            task_name: String::new(),
            mode: Mode::Update,
            status: RunStatus::Running,
            started_at: 0,
            ended_at: None,
            elapsed: 0.0,
            links: Vec::new(),
            totals: Totals::default(),
            errors: Vec::new(),
            errors_total: 0,
        }
    }
}

/// Résumé de la dernière exécution d'une tâche. Il survit à l'effacement de l'historique :
/// la carte de la tâche continue d'afficher « Terminée il y a… ».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub status: RunStatus,
    pub started_at: i64,
}

fn last_runs_path(store: &Store) -> PathBuf {
    store.root().join("last-runs.json")
}

fn read_last_runs(store: &Store) -> HashMap<String, LastRun> {
    fs::read(last_runs_path(store))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Retient l'exécution si elle est la plus récente connue pour sa tâche.
pub fn remember(store: &Store, records: &[RunRecord]) -> Result<()> {
    let mut known = read_last_runs(store);
    let mut changed = false;
    for record in records.iter().filter(|r| !r.task_id.is_empty() && !r.status.is_active()) {
        let newer = known.get(&record.task_id).is_none_or(|last| last.started_at <= record.started_at);
        if newer {
            known.insert(
                record.task_id.clone(),
                LastRun {
                    status: record.status,
                    started_at: record.started_at,
                },
            );
            changed = true;
        }
    }
    if changed {
        atomic_write(&last_runs_path(store), &serde_json::to_vec_pretty(&known)?)?;
    }
    Ok(())
}

/// Dernière exécution de chaque tâche, d'après l'historique et ce qui en a été retenu.
pub fn last_runs(store: &Store) -> Result<HashMap<String, LastRun>> {
    let mut known = read_last_runs(store);
    for record in list(store)? {
        if record.task_id.is_empty() {
            continue;
        }
        let newer = known.get(&record.task_id).is_none_or(|last| last.started_at < record.started_at);
        if newer {
            known.insert(
                record.task_id.clone(),
                LastRun {
                    status: record.status,
                    started_at: record.started_at,
                },
            );
        }
    }
    Ok(known)
}

fn run_dir(store: &Store, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return fail_text("err.bad-id");
    }
    Ok(store.runs_dir().join(id))
}

pub fn save(store: &Store, record: &RunRecord) -> Result<()> {
    atomic_write(&run_dir(store, &record.id)?.join("run.json"), &serde_json::to_vec_pretty(record)?)
}

pub fn get(store: &Store, id: &str) -> Result<RunRecord> {
    Ok(serde_json::from_slice(&fs::read(run_dir(store, id)?.join("run.json"))?)?)
}

/// Les exécutions, les plus récentes d'abord, sans leurs listes d'erreurs.
pub fn list(store: &Store) -> Result<Vec<RunRecord>> {
    let mut records = Vec::new();
    for entry in fs::read_dir(store.runs_dir())?.flatten() {
        let Ok(bytes) = fs::read(entry.path().join("run.json")) else { continue };
        let Ok(mut record) = serde_json::from_slice::<RunRecord>(&bytes) else {
            continue;
        };
        record.errors.clear();
        records.push(record);
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.started_at));
    Ok(records)
}

pub fn delete(store: &Store, id: &str) -> Result<()> {
    let dir = run_dir(store, id)?;
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Efface tous les rapports et journaux, sauf celui de l'exécution en cours, et en renvoie le nombre.
/// Un dossier qui résiste n'empêche pas d'effacer les autres.
pub fn clear(store: &Store, keep: Option<&str>) -> Result<usize> {
    remember(store, &list(store)?)?;
    let mut removed = 0;
    let mut failure = None;
    for entry in fs::read_dir(store.runs_dir())?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if keep == Some(id.as_str()) || !entry.path().join("run.json").exists() {
            continue;
        }
        match delete(store, &id) {
            Ok(()) => removed += 1,
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(removed),
    }
}

/// Au démarrage, une exécution encore « en cours » n'a pas pu se terminer.
pub fn mark_interrupted(store: &Store) -> Result<usize> {
    let mut count = 0;
    for mut record in list(store)? {
        if record.status.is_active() {
            let full = get(store, &record.id)?;
            record.errors = full.errors;
            record.status = RunStatus::Interrupted;
            for link in &mut record.links {
                link.current.clear();
                if matches!(link.phase, LinkPhase::Copying | LinkPhase::Verifying) {
                    link.phase = LinkPhase::Failed;
                    link.message = Some(ui_text("msg.interrupted", &[]));
                }
            }
            save(store, &record)?;
            count += 1;
        }
    }
    Ok(count)
}

pub struct Journal {
    file: Mutex<BufWriter<fs::File>>,
}

impl Journal {
    pub fn create(store: &Store, id: &str) -> Result<Self> {
        let dir = run_dir(store, id)?;
        fs::create_dir_all(&dir)?;
        Ok(Self {
            file: Mutex::new(BufWriter::new(fs::File::create(dir.join("journal.log"))?)),
        })
    }

    pub fn write(&self, line: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub total: usize,
    pub lines: Vec<String>,
}

pub fn read_log(store: &Store, id: &str, search: &str, offset: usize, limit: usize) -> Result<LogPage> {
    let path = run_dir(store, id)?.join("journal.log");
    let needle = search.to_lowercase();
    let mut total = 0;
    let mut lines = Vec::new();
    let Ok(file) = fs::File::open(&path) else {
        return Ok(LogPage { total: 0, lines });
    };
    for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
        if !needle.is_empty() && !line.to_lowercase().contains(&needle) {
            continue;
        }
        if total >= offset && lines.len() < limit {
            lines.push(line);
        }
        total += 1;
    }
    Ok(LogPage { total, lines })
}

pub fn log_path(store: &Store, id: &str) -> Result<PathBuf> {
    Ok(run_dir(store, id)?.join("journal.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_left_open_is_marked_interrupted_and_never_resumed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let record = RunRecord {
            id: "abc123".into(),
            status: RunStatus::Running,
            links: vec![LinkRecord {
                id: "l".into(),
                phase: LinkPhase::Copying,
                ..LinkRecord::default()
            }],
            ..RunRecord::default()
        };
        save(&store, &record).unwrap();
        assert_eq!(mark_interrupted(&store).unwrap(), 1);
        let after = get(&store, "abc123").unwrap();
        assert_eq!(after.status, RunStatus::Interrupted);
        assert_eq!(after.links[0].phase, LinkPhase::Failed);
        assert_eq!(mark_interrupted(&store).unwrap(), 0);
    }

    #[test]
    fn journal_is_searchable_by_page() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let journal = Journal::create(&store, "run1").unwrap();
        for i in 0..50 {
            journal.write(&format!("ligne {i}"));
        }
        journal.write("ERREUR rare");
        journal.flush();
        let page = read_log(&store, "run1", "", 45, 3).unwrap();
        assert_eq!((page.total, page.lines.len()), (51, 3));
        assert_eq!(read_log(&store, "run1", "erreur", 0, 10).unwrap().total, 1);
    }

    #[test]
    fn clearing_spares_the_running_operation_and_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        for id in ["ancien1", "ancien2", "encours"] {
            save(
                &store,
                &RunRecord {
                    id: id.into(),
                    ..RunRecord::default()
                },
            )
            .unwrap();
        }
        fs::create_dir_all(store.runs_dir().join("inconnu")).unwrap();
        let task = store
            .save_task(crate::model::Task {
                name: "Conservée".into(),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(clear(&store, Some("encours")).unwrap(), 2);
        let left: Vec<String> = list(&store).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(left, ["encours"]);
        assert!(store.runs_dir().join("inconnu").exists(), "seuls les rapports sont effacés");
        assert_eq!(store.get_task(&task.id).unwrap().name, "Conservée");

        assert_eq!(clear(&store, None).unwrap(), 1);
        assert!(list(&store).unwrap().is_empty());
    }

    #[test]
    fn erasing_the_history_keeps_when_each_task_last_ran() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let run = |id: &str, task: &str, at: i64, status: RunStatus| RunRecord {
            id: id.into(),
            task_id: task.into(),
            started_at: at,
            status,
            ..RunRecord::default()
        };
        save(&store, &run("a1", "a", 100, RunStatus::Failed)).unwrap();
        save(&store, &run("a2", "a", 200, RunStatus::Completed)).unwrap();
        save(&store, &run("b1", "b", 150, RunStatus::Cancelled)).unwrap();

        clear(&store, None).unwrap();
        assert!(list(&store).unwrap().is_empty());
        let last = last_runs(&store).unwrap();
        assert_eq!(
            last["a"],
            LastRun {
                status: RunStatus::Completed,
                started_at: 200
            }
        );
        assert_eq!(last["b"].status, RunStatus::Cancelled);

        save(&store, &run("a3", "a", 300, RunStatus::CompletedWithErrors)).unwrap();
        assert_eq!(last_runs(&store).unwrap()["a"].started_at, 300, "une exécution plus récente l'emporte");
    }

    #[test]
    fn run_ids_cannot_escape_the_history_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        assert!(delete(&store, "../tasks").is_err());
    }
}
