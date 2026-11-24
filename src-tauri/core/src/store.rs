//! Persistance locale, indépendante du dossier d'installation.
//!
//! Une tâche est un fichier JSON lisible. Chaque écriture est atomique et conserve la version
//! précédente : ni une coupure de courant ni une mise à jour ne peuvent faire perdre une tâche.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{Task, TASK_SCHEMA};
use crate::{fail_text, new_id, now_ms, Result};

const EXPORT_FORMAT: &str = "chud-copy-tasks";

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    pub tasks: Vec<Task>,
    pub broken: Vec<BrokenFile>,
}

#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    tasks: Vec<Task>,
}

/// Dossier de données de l'utilisateur, hors du dossier d'installation.
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("CHUD_COPY_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("APPDATA") {
            return PathBuf::from(dir).join("ChudCopy");
        }
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("chud-copy");
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".local/share/chud-copy"),
        None => std::env::temp_dir().join("chud-copy"),
    }
}

/// Écrit dans un fichier voisin, le force sur le disque puis le met en place d'un seul geste.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("{}.tmp", new_id()));
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        let _ = fs::copy(path, path.with_extension("bak"));
    }
    fs::rename(&temp, path)?;
    Ok(())
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        for dir in ["tasks", "runs", "tmp"] {
            fs::create_dir_all(root.join(dir))?;
        }
        let store = Self { root };
        store.purge_tmp();
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn purge_tmp(&self) {
        if let Ok(entries) = fs::read_dir(self.tmp_dir()) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn task_path(&self, id: &str) -> Result<PathBuf> {
        // L'identifiant vient d'un fichier importable : il ne doit pas pouvoir sortir du dossier.
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return fail_text("err.bad-id");
        }
        Ok(self.root.join("tasks").join(format!("{id}.json")))
    }

    fn read_task(path: &Path) -> Result<Task> {
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let mut task: Task = serde_json::from_value(migrate(raw))?;
        task.schema = TASK_SCHEMA;
        Ok(task)
    }

    pub fn load_tasks(&self) -> Result<TaskList> {
        let mut tasks = Vec::new();
        let mut broken = Vec::new();
        for entry in fs::read_dir(self.root.join("tasks"))?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match Self::read_task(&path).or_else(|first| Self::read_task(&path.with_extension("bak")).map_err(|_| first)) {
                Ok(task) => tasks.push(task),
                // Un fichier illisible n'est ni effacé ni réécrit : il reste récupérable à la main.
                Err(e) => broken.push(BrokenFile {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                }),
            }
        }
        tasks.sort_by_key(|task| std::cmp::Reverse(task.updated_at));
        Ok(TaskList { tasks, broken })
    }

    pub fn get_task(&self, id: &str) -> Result<Task> {
        Self::read_task(&self.task_path(id)?)
    }

    pub fn save_task(&self, mut task: Task) -> Result<Task> {
        let now = now_ms();
        if task.id.is_empty() {
            task.id = new_id();
            task.created_at = now;
        }
        task.schema = TASK_SCHEMA;
        task.updated_at = now;
        // La couleur finit dans une feuille de style : seule une couleur hexadécimale est gardée.
        let valid = |c: &String| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit());
        task.color = task.color.filter(valid);
        atomic_write(&self.task_path(&task.id)?, &serde_json::to_vec_pretty(&task)?)?;
        Ok(task)
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        let path = self.task_path(id)?;
        if path.exists() {
            // Une tâche supprimée par erreur reste récupérable dans ce dossier.
            let removed = self.root.join("deleted");
            fs::create_dir_all(&removed)?;
            fs::rename(&path, removed.join(format!("{id}.json")))?;
        }
        let _ = fs::remove_file(path.with_extension("bak"));
        Ok(())
    }

    pub fn duplicate_task(&self, id: &str) -> Result<Task> {
        let mut copy = self.get_task(id)?;
        copy.id = String::new();
        copy.name = format!("{} (copie)", copy.name);
        self.save_task(copy)
    }

    pub fn export_tasks(&self, ids: &[String], path: &Path) -> Result<usize> {
        let mut tasks = Vec::new();
        for id in ids {
            tasks.push(self.get_task(id)?);
        }
        let file = ExportFile {
            format: EXPORT_FORMAT.to_string(),
            version: 1,
            tasks,
        };
        atomic_write(path, &serde_json::to_vec_pretty(&file)?)?;
        Ok(file.tasks.len())
    }

    pub fn import_tasks(&self, path: &Path) -> Result<Vec<Task>> {
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let incoming: Vec<serde_json::Value> = if raw.get("format").and_then(|f| f.as_str()) == Some(EXPORT_FORMAT) {
            raw.get("tasks").and_then(|t| t.as_array()).cloned().unwrap_or_default()
        } else if raw.get("sources").is_some() {
            vec![raw]
        } else {
            return fail_text("err.not-export");
        };

        let existing = self.load_tasks()?.tasks;
        let mut imported = Vec::new();
        for value in incoming {
            let mut task: Task = serde_json::from_value(migrate(value))?;
            // Importer n'écrase jamais une tâche existante.
            task.id = String::new();
            // Les disques reconnus sur un autre ordinateur n'ont pas de sens ici : ils seront réappris.
            for endpoint in task.sources.iter_mut().chain(task.destinations.iter_mut()) {
                endpoint.identity = None;
            }
            if existing.iter().any(|t| t.name == task.name) {
                task.name = format!("{} (importée)", task.name);
            }
            imported.push(self.save_task(task)?);
        }
        Ok(imported)
    }

    pub fn load_settings(&self) -> serde_json::Value {
        fs::read(self.root.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    pub fn save_settings(&self, value: &serde_json::Value) -> Result<()> {
        atomic_write(&self.root.join("settings.json"), &serde_json::to_vec_pretty(value)?)
    }
}

/// Fait passer un fichier ancien au schéma courant. Le point d'accroche des futures évolutions.
fn migrate(value: serde_json::Value) -> serde_json::Value {
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Endpoint, Mode};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn sample(name: &str) -> Task {
        Task {
            name: name.into(),
            mode: Mode::Mirror,
            sources: vec![Endpoint {
                id: "s".into(),
                path: "Q:\\Absent\\Photos".into(),
                ..Default::default()
            }],
            ..Task::default()
        }
    }

    #[test]
    fn tasks_survive_a_restart() {
        let (dir, store) = store();
        let saved = store.save_task(sample("Sauvegarde PC")).unwrap();
        drop(store);
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let tasks = reopened.load_tasks().unwrap().tasks;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0], saved);
    }

    #[test]
    fn only_a_real_colour_is_kept() {
        let (_dir, store) = store();
        let mut task = sample("Teintée");
        task.color = Some("#3A7BFF".into());
        assert_eq!(store.save_task(task.clone()).unwrap().color.as_deref(), Some("#3A7BFF"));
        task.color = Some("red;}body{display:none".into());
        assert_eq!(store.save_task(task).unwrap().color, None);
    }

    #[test]
    fn a_missing_disk_keeps_its_path() {
        let (_dir, store) = store();
        store.save_task(sample("Disque absent")).unwrap();
        let task = &store.load_tasks().unwrap().tasks[0];
        assert_eq!(task.sources[0].path, "Q:\\Absent\\Photos");
    }

    #[test]
    fn a_file_from_an_older_version_gets_defaults() {
        let (dir, store) = store();
        let old = r#"{"id":"ancien","name":"Ancienne tâche","mode":"update"}"#;
        fs::write(dir.path().join("tasks/ancien.json"), old).unwrap();
        let task = store.get_task("ancien").unwrap();
        assert_eq!(task.options.transfers, 4);
        assert!(task.filters.exclude_system);
    }

    #[test]
    fn a_corrupted_task_falls_back_on_its_previous_version() {
        let (dir, store) = store();
        let saved = store.save_task(sample("Une")).unwrap();
        store.save_task(saved.clone()).unwrap();
        fs::write(dir.path().join(format!("tasks/{}.json", saved.id)), "{ tronqué").unwrap();
        let list = store.load_tasks().unwrap();
        assert_eq!(list.tasks.len(), 1);
        assert!(list.broken.is_empty());
    }

    #[test]
    fn an_unreadable_file_is_reported_and_left_in_place() {
        let (dir, store) = store();
        let path = dir.path().join("tasks/casse.json");
        fs::write(&path, "pas du json").unwrap();
        let list = store.load_tasks().unwrap();
        assert_eq!(list.broken.len(), 1);
        assert!(path.exists());
    }

    #[test]
    fn export_then_import_never_overwrites() {
        let (dir, store) = store();
        let saved = store.save_task(sample("Archive")).unwrap();
        let file = dir.path().join("export.chudcopy");
        assert_eq!(store.export_tasks(std::slice::from_ref(&saved.id), &file).unwrap(), 1);
        let imported = store.import_tasks(&file).unwrap();
        assert_ne!(imported[0].id, saved.id);
        assert_eq!(imported[0].name, "Archive (importée)");
        assert_eq!(store.load_tasks().unwrap().tasks.len(), 2);
    }

    #[test]
    fn deleted_tasks_stay_recoverable() {
        let (dir, store) = store();
        let saved = store.save_task(sample("Temporaire")).unwrap();
        store.delete_task(&saved.id).unwrap();
        assert!(store.load_tasks().unwrap().tasks.is_empty());
        assert!(dir.path().join(format!("deleted/{}.json", saved.id)).exists());
    }

    #[test]
    fn ids_cannot_escape_the_data_folder() {
        let (_dir, store) = store();
        assert!(store.get_task("../../secret").is_err());
    }

    #[test]
    fn rejects_foreign_json() {
        let (dir, store) = store();
        let file = dir.path().join("autre.json");
        fs::write(&file, "{\"hello\":1}").unwrap();
        assert!(store.import_tasks(&file).is_err());
    }
}
