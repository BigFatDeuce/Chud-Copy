//! Application Tauri : expose le cœur à l'interface, sans logique métier propre.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chud_core::cleanup::{self, CleanupItem, CleanupOptions, CleanupReport, Place, TrashOutcome};
use chud_core::drives::{self, DriveCheck, DriveIdentity};
use chud_core::engine::rclone::Rclone;
use chud_core::engine::{Control, Engine};
use chud_core::filters;
use chud_core::health::{self, HealthReport};
use chud_core::history::{self, LastRun, LogPage, RunRecord, RunStatus};
use chud_core::integrity::{self, IntegrityProgress, IntegrityReport, Reference};
use chud_core::model::Task;
use chud_core::network::{self, NetworkOptions, NetworkReport};
use chud_core::plan::{self, Plan, PlanPage, PlanProgress, PlanQuery};
use chud_core::queue::{Queue, QueueView};
use chud_core::runner::{self, Emit, RunHandle};
use chud_core::store::{self, Store, TaskList};
use chud_core::recycle::Recycled;
use chud_core::tools::{self, ArchiveVersion, ArchivedFile, CompareReport, DuplicateGroup, DuplicateReport, ReadReport, SpaceNode, ToolProgress};
use chud_core::ui_text;
use chud_core::validate::{self, Report};
use chud_core::volumes::{self, Disk};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    store: Store,
    engine: Arc<dyn Engine>,
    plans: Mutex<HashMap<String, Arc<Plan>>>,
    /// Les analyses en cours, une au plus par tâche.
    analyses: Mutex<HashMap<String, Control>>,
    run: Mutex<Option<RunHandle>>,
    queue: Mutex<Queue>,
    /// Les contrôles d'intégrité en cours, un au plus par dossier.
    checks: Mutex<HashMap<String, Control>>,
    /// Les outils en cours, par identifiant choisi par l'interface.
    jobs: Mutex<HashMap<String, Control>>,
    /// Sérialise les démarrages : un seul peut constater que la voie est libre.
    launching: Mutex<()>,
}

type CommandResult<T> = Result<T, String>;

/// Qui dessine la barre de titre et ses boutons. Sous Linux c'est le bureau : sa barre native
/// garantit le déplacement et le redimensionnement sur tous les environnements. Ailleurs c'est
/// l'interface. Elle l'apprend par `AppInfo::chrome` et n'a donc jamais à le deviner : une fenêtre
/// sans aucun bouton pour la fermer n'est plus possible.
const NATIVE_CHROME: bool = cfg!(target_os = "linux");

/// Complète la disposition des boutons de la barre de titre (« gauche:droite », noms séparés par
/// des virgules) pour qu'elle offre toujours réduire, agrandir et fermer. Certains bureaux, GNOME
/// en tête, ne demandent que « fermer ». Les boutons déjà placés par le bureau gardent leur place ;
/// les manquants rejoignent le côté de « fermer », qui reste au bord de la fenêtre.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn complete_window_buttons(layout: &str) -> String {
    let (left, right) = layout.split_once(':').unwrap_or((layout, ""));
    let side = |buttons: &str| buttons.split(',').map(str::trim).filter(|b| !b.is_empty()).map(String::from).collect::<Vec<_>>();
    let (mut left, mut right) = (side(left), side(right));
    let missing: Vec<String> = ["minimize", "maximize", "close"]
        .into_iter()
        .filter(|button| !left.iter().chain(right.iter()).any(|b| b == button))
        .map(String::from)
        .collect();
    if left.iter().any(|b| b == "close") {
        left.extend(missing);
    } else {
        right.retain(|b| b != "close");
        right.extend(missing.into_iter().filter(|b| b != "close"));
        right.push("close".into());
    }
    format!("{}:{}", left.join(","), right.join(","))
}

fn text<E: ToString>(error: E) -> String {
    error.to_string()
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> CommandResult<T> + Send + 'static) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(text)?
}

/// Exécute une commande hors du fil de l'interface, avec l'état de l'application. Tout accès à un
/// disque passe par là : un partage réseau lent ou injoignable peut répondre en plusieurs dizaines
/// de secondes, et le fil de l'interface ne doit jamais l'attendre.
async fn off_ui<T: Send + 'static>(app: AppHandle, work: impl FnOnce(&AppHandle, &AppState) -> CommandResult<T> + Send + 'static) -> CommandResult<T> {
    blocking(move || {
        let state = app.state::<AppState>();
        work(&app, &state)
    })
    .await
}

/// La fenêtre principale, propriétaire des éventuels avertissements du système.
fn owner_window(app: &AppHandle) -> Option<isize> {
    #[cfg(windows)]
    {
        app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

fn engine_binary() -> PathBuf {
    let name = if cfg!(windows) { "rclone.exe" } else { "rclone" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn run_is_active(state: &AppState) -> bool {
    state.run.lock().unwrap().as_ref().is_some_and(|run| !run.is_finished())
}

/// Son de fin d'opération joué par le système, sans passer par WebKitGTK (voir l'interface).
/// Renvoie vrai si un lecteur a été lancé. Les sons sont déjà ramenés à 30 % de leur volume.
#[tauri::command]
fn play_sound(kind: String) -> bool {
    #[cfg(target_os = "linux")]
    {
        let (name, bytes): (&str, &[u8]) = match kind.as_str() {
            "good" => ("good.wav", include_bytes!("../sounds/good.wav")),
            "bad" => ("bad.wav", include_bytes!("../sounds/bad.wav")),
            _ => return false,
        };
        let dir = std::env::temp_dir().join("chud-copy-sounds");
        let file = dir.join(name);
        let present = std::fs::metadata(&file).is_ok_and(|meta| meta.len() == bytes.len() as u64);
        if !present && (std::fs::create_dir_all(&dir).is_err() || std::fs::write(&file, bytes).is_err()) {
            return false;
        }
        // Chaque lecteur est essayé tour à tour, dans un fil à part : un lecteur absent ou sans serveur son passe la main.
        std::thread::spawn(move || {
            for player in ["pw-play", "paplay", "aplay"] {
                let mut command = std::process::Command::new(player);
                if player == "aplay" {
                    command.arg("-q");
                }
                command
                    .arg(&file)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                // Les lecteurs du système ne doivent pas charger les bibliothèques embarquées par l'AppImage.
                for variable in [
                    "LD_LIBRARY_PATH",
                    "LD_PRELOAD",
                    "GST_PLUGIN_PATH",
                    "GST_PLUGIN_SYSTEM_PATH",
                    "GST_PLUGIN_SYSTEM_PATH_1_0",
                    "GST_PLUGIN_SCANNER",
                    "GIO_MODULE_DIR",
                    "GDK_PIXBUF_MODULE_FILE",
                ] {
                    command.env_remove(variable);
                }
                if command.status().is_ok_and(|status| status.success()) {
                    break;
                }
            }
        });
        true
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = kind;
        false
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    engine: String,
    engine_version: String,
    data_dir: String,
    platform: String,
    /// « native » ou « custom », voir `NATIVE_CHROME`.
    chrome: &'static str,
    interrupted: Vec<String>,
}

#[tauri::command]
async fn app_info(app: AppHandle, state: State<'_, AppState>) -> CommandResult<AppInfo> {
    let engine = state.engine.clone();
    let data_dir = state.store.root().display().to_string();
    let store = state.store.clone();
    let version = app.package_info().version.to_string();
    blocking(move || {
        let interrupted = history::list(&store)
            .map_err(text)?
            .into_iter()
            .filter(|r| r.status == history::RunStatus::Interrupted)
            .map(|r| r.id)
            .collect();
        Ok(AppInfo {
            version,
            engine: engine.name(),
            engine_version: engine.version().unwrap_or_else(|e| e.to_string()),
            data_dir,
            platform: std::env::consts::OS.to_string(),
            chrome: if NATIVE_CHROME { "native" } else { "custom" },
            interrupted,
        })
    })
    .await
}

#[tauri::command]
async fn tasks_list(app: AppHandle) -> CommandResult<TaskList> {
    off_ui(app, |_, state| state.store.load_tasks().map_err(text)).await
}

/// Enregistre une tâche : l'identité d'un disque déjà reconnu suit son chemin, celle d'un chemin
/// changé est oubliée puis réapprise si le dossier existe.
#[tauri::command]
async fn task_save(state: State<'_, AppState>, task: Task) -> CommandResult<Task> {
    let store = state.store.clone();
    blocking(move || {
        let mut task = task;
        match (!task.id.is_empty()).then(|| store.get_task(&task.id).ok()).flatten() {
            Some(previous) => drives::carry_over(&mut task, &previous),
            None => {
                for endpoint in task.sources.iter_mut().chain(task.destinations.iter_mut()) {
                    endpoint.identity = None;
                }
            }
        }
        drives::learn_missing(&mut task);
        store.save_task(task).map_err(text)
    })
    .await
}

/// Mémorise les disques encore inconnus d'une tâche avant une analyse ou une copie.
fn learn_drives(store: &Store, task: Task) -> Task {
    let mut task = task;
    if drives::learn_missing(&mut task) {
        if let Ok(saved) = store.save_task(task.clone()) {
            return saved;
        }
    }
    task
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriveRow {
    task_id: String,
    task_name: String,
    endpoint_id: String,
    /// « source » ou « destination ».
    role: String,
    path: String,
    identity: Option<DriveIdentity>,
    check: DriveCheck,
}

/// Chaque source et destination de chaque tâche, avec son disque mémorisé et celui trouvé.
#[tauri::command]
async fn drives_status(state: State<'_, AppState>) -> CommandResult<Vec<DriveRow>> {
    let store = state.store.clone();
    blocking(move || {
        let tasks = store.load_tasks().map_err(text)?.tasks;
        let mut rows = Vec::new();
        for task in &tasks {
            for (role, endpoints) in [("source", &task.sources), ("destination", &task.destinations)] {
                for endpoint in endpoints.iter() {
                    rows.push(DriveRow {
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        endpoint_id: endpoint.id.clone(),
                        role: role.into(),
                        path: endpoint.path.clone(),
                        identity: endpoint.identity.clone(),
                        check: DriveCheck::Unknown,
                    });
                }
            }
        }
        // Chaque disque est interrogé en parallèle : un partage injoignable ne retarde pas les autres.
        let checks: Vec<_> = rows
            .iter()
            .map(|row| {
                let identity = row.identity.clone();
                let path = row.path.clone();
                std::thread::spawn(move || drives::check_in_time(identity.as_ref(), Path::new(&path)))
            })
            .collect();
        for (row, check) in rows.iter_mut().zip(checks) {
            row.check = check.join().unwrap_or(DriveCheck::Absent);
        }
        Ok(rows)
    })
    .await
}

/// Confirmation explicite de l'utilisateur : le disque présent à cet endroit est le bon.
#[tauri::command]
async fn drive_accept(state: State<'_, AppState>, task_id: String, endpoint_id: String) -> CommandResult<Task> {
    let store = state.store.clone();
    blocking(move || {
        let mut task = store.get_task(&task_id).map_err(text)?;
        let endpoint = task
            .sources
            .iter_mut()
            .chain(task.destinations.iter_mut())
            .find(|e| e.id == endpoint_id)
            .ok_or_else(|| ui_text("err.drive-absent", &[]))?;
        // Un dossier absent ferait mémoriser le disque qui le contiendrait, souvent le disque système.
        if !Path::new(&endpoint.path).is_dir() {
            return Err(ui_text("err.drive-absent", &[]));
        }
        endpoint.identity = Some(drives::identify(Path::new(&endpoint.path)).ok_or_else(|| ui_text("err.drive-absent", &[]))?);
        store.save_task(task).map_err(text)
    })
    .await
}

#[tauri::command]
async fn task_delete(app: AppHandle, id: String) -> CommandResult<()> {
    off_ui(app, move |_, state| state.store.delete_task(&id).map_err(text)).await
}

#[tauri::command]
async fn task_duplicate(app: AppHandle, id: String) -> CommandResult<Task> {
    off_ui(app, move |_, state| state.store.duplicate_task(&id).map_err(text)).await
}

#[tauri::command]
async fn tasks_export(app: AppHandle, ids: Vec<String>, path: String) -> CommandResult<usize> {
    off_ui(app, move |_, state| state.store.export_tasks(&ids, Path::new(&path)).map_err(text)).await
}

#[tauri::command]
async fn tasks_import(app: AppHandle, path: String) -> CommandResult<Vec<Task>> {
    off_ui(app, move |_, state| state.store.import_tasks(Path::new(&path)).map_err(text)).await
}

#[tauri::command]
async fn task_validate(task: Task) -> CommandResult<Report> {
    blocking(move || Ok(validate::validate(&task))).await
}

/// Dernière liste des disques, et vrai pendant qu'une lecture est en cours.
static DISKS: Mutex<(Vec<Disk>, bool)> = Mutex::new((Vec::new(), false));

/// Liste des disques, relue au plus une fois à la fois. Un montage réseau figé (NFS, SMB) peut
/// bloquer sa lecture indéfiniment : l'interface, qui la redemande toutes les 15 s, reçoit alors
/// la dernière liste connue au lieu d'empiler des lectures qui ne finissent jamais.
#[tauri::command]
async fn disks_list() -> CommandResult<Vec<Disk>> {
    {
        let mut cache = DISKS.lock().unwrap_or_else(|e| e.into_inner());
        if cache.1 {
            return Ok(cache.0.clone());
        }
        cache.1 = true;
    }
    let list = blocking(|| Ok(volumes::list())).await;
    let mut cache = DISKS.lock().unwrap_or_else(|e| e.into_inner());
    cache.1 = false;
    if let Ok(list) = &list {
        cache.0 = list.clone();
    }
    list
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisProgress {
    task_id: String,
    #[serde(flatten)]
    progress: PlanProgress,
}

#[tauri::command]
async fn plan_start(app: AppHandle, state: State<'_, AppState>, task_id: String) -> CommandResult<SharedPlan> {
    let task = state.store.get_task(&task_id).map_err(text)?;
    let store = state.store.clone();
    let task = blocking(move || Ok(learn_drives(&store, task))).await?;
    let control = Control::default();
    {
        let mut analyses = state.analyses.lock().unwrap();
        if analyses.contains_key(&task_id) {
            return Err(ui_text("err.busy-analysis", &[]));
        }
        analyses.insert(task_id.clone(), control.clone());
    }
    let engine = state.engine.clone();
    let events = app.clone();
    let reported = task_id.clone();
    let result = blocking(move || {
        let emit = |progress: PlanProgress| {
            let _ = events.emit(
                "plan:progress",
                AnalysisProgress {
                    task_id: reported.clone(),
                    progress,
                },
            );
        };
        plan::analyze_task(engine.as_ref(), &task, &control, &emit).map_err(text)
    })
    .await;
    state.analyses.lock().unwrap().remove(&task_id);

    // Le plan, avec la liste complète des fichiers, est partagé et non copié : seul son résumé
    // part vers l'interface, la liste étant consultée page par page.
    let plan = Arc::new(result?);
    state.plans.lock().unwrap().insert(plan.task_id.clone(), plan.clone());
    Ok(SharedPlan(plan))
}

/// Un plan partagé, envoyé à l'interface tel qu'il se sérialise (sans la liste des fichiers).
struct SharedPlan(Arc<Plan>);

impl Serialize for SharedPlan {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

#[tauri::command]
fn plan_cancel(state: State<'_, AppState>, task_id: String) {
    if let Some(control) = state.analyses.lock().unwrap().get(&task_id) {
        control.cancel();
    }
}

#[tauri::command]
fn plan_forget(state: State<'_, AppState>, task_id: String) {
    state.plans.lock().unwrap().remove(&task_id);
}

#[tauri::command]
async fn plan_query(state: State<'_, AppState>, task_id: String, query: PlanQuery) -> CommandResult<PlanPage> {
    let plan = state.plans.lock().unwrap().get(&task_id).cloned();
    let Some(plan) = plan else {
        return Err(ui_text("err.plan-gone", &[]));
    };
    blocking(move || Ok(plan::query(&plan, &query))).await
}

fn launch(app: &AppHandle, state: &AppState, task_id: &str, plan: Option<Arc<Plan>>) -> CommandResult<String> {
    let task = learn_drives(&state.store, state.store.get_task(task_id).map_err(text)?);
    let events = app.clone();
    let emit: Emit = Arc::new(move |record: &RunRecord| {
        let _ = events.emit("run:update", record);
    });
    let follow = app.clone();
    let then = move |status: RunStatus| {
        if status == RunStatus::Cancelled {
            let state = follow.state::<AppState>();
            state.queue.lock().unwrap().hold();
        }
        start_next(&follow);
    };
    let handle = runner::start_then(state.engine.clone(), state.store.clone(), task, plan, emit, then).map_err(text)?;
    let id = handle.run_id.clone();
    *state.run.lock().unwrap() = Some(handle);
    Ok(id)
}

fn publish_queue(app: &AppHandle) {
    let view = app.state::<AppState>().queue.lock().unwrap().view();
    let _ = app.emit("queue:update", view);
}

/// Démarre la prochaine opération de la file si la voie est libre.
fn start_next(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _launching = state.launching.lock().unwrap();
    while !run_is_active(&state) {
        let Some(item) = state.queue.lock().unwrap().take_next() else {
            break;
        };
        if let Err(reason) = launch(app, &state, &item.task_id, item.plan.clone()) {
            state.queue.lock().unwrap().refuse(item, reason);
        }
    }
    publish_queue(app);
}

#[tauri::command]
async fn run_start(app: AppHandle, task_id: String, use_plan: bool) -> CommandResult<String> {
    off_ui(app, move |app, state| {
        let _launching = state.launching.lock().unwrap();
        if run_is_active(state) {
            return Err(ui_text("err.busy-run", &[]));
        }
        let plan = if use_plan { state.plans.lock().unwrap().get(&task_id).cloned() } else { None };
        launch(app, state, &task_id, plan)
    })
    .await
}

#[tauri::command]
fn queue_get(state: State<'_, AppState>) -> QueueView {
    state.queue.lock().unwrap().view()
}

/// Un miroir n'entre dans la file qu'avec une analyse valable, dont les suppressions restent
/// recomparées juste avant d'écrire.
#[tauri::command]
async fn queue_add(app: AppHandle, task_id: String, use_plan: bool) -> CommandResult<()> {
    off_ui(app, move |app, state| {
        let task = state.store.get_task(&task_id).map_err(text)?;
        let plan = if use_plan { state.plans.lock().unwrap().get(&task_id).cloned() } else { None };
        runner::validated_deletions(&task, plan.as_ref()).map_err(text)?;
        state.queue.lock().unwrap().add(&task, plan).map_err(text)?;
        start_next(app);
        Ok(())
    })
    .await
}

#[tauri::command]
fn queue_remove(app: AppHandle, state: State<'_, AppState>, id: String) {
    state.queue.lock().unwrap().remove(&id);
    publish_queue(&app);
}

#[tauri::command]
fn queue_shift(app: AppHandle, state: State<'_, AppState>, id: String, earlier: bool) {
    state.queue.lock().unwrap().shift(&id, earlier);
    publish_queue(&app);
}

#[tauri::command]
fn queue_clear(app: AppHandle, state: State<'_, AppState>) {
    state.queue.lock().unwrap().clear();
    publish_queue(&app);
}

#[tauri::command]
async fn queue_resume(app: AppHandle) -> CommandResult<()> {
    off_ui(app, |app, state| {
        state.queue.lock().unwrap().resume();
        start_next(app);
        Ok(())
    })
    .await
}

#[tauri::command]
fn run_pause(state: State<'_, AppState>) {
    if let Some(run) = state.run.lock().unwrap().as_ref() {
        run.control.pause();
    }
}

#[tauri::command]
fn run_resume(state: State<'_, AppState>) {
    if let Some(run) = state.run.lock().unwrap().as_ref() {
        run.control.resume();
    }
}

#[tauri::command]
fn run_cancel(state: State<'_, AppState>) {
    if let Some(run) = state.run.lock().unwrap().as_ref() {
        run.control.cancel();
    }
}

#[tauri::command]
fn run_current(state: State<'_, AppState>) -> Option<RunRecord> {
    state.run.lock().unwrap().as_ref().filter(|run| !run.is_finished()).map(|run| run.snapshot())
}

#[tauri::command]
async fn history_list(app: AppHandle) -> CommandResult<Vec<RunRecord>> {
    off_ui(app, |_, state| history::list(&state.store).map_err(text)).await
}

#[tauri::command]
async fn history_get(app: AppHandle, id: String) -> CommandResult<RunRecord> {
    off_ui(app, move |_, state| history::get(&state.store, &id).map_err(text)).await
}

#[tauri::command]
async fn history_delete(app: AppHandle, id: String) -> CommandResult<()> {
    off_ui(app, move |_, state| history::delete(&state.store, &id).map_err(text)).await
}

#[tauri::command]
async fn history_clear(app: AppHandle) -> CommandResult<usize> {
    off_ui(app, |_, state| {
        let running = state
            .run
            .lock()
            .unwrap()
            .as_ref()
            .filter(|run| !run.is_finished())
            .map(|run| run.run_id.clone());
        history::clear(&state.store, running.as_deref()).map_err(text)
    })
    .await
}

#[tauri::command]
async fn tasks_last_runs(app: AppHandle) -> CommandResult<HashMap<String, LastRun>> {
    off_ui(app, |_, state| history::last_runs(&state.store).map_err(text)).await
}

#[tauri::command]
fn junk_list() -> Vec<String> {
    filters::junk_list()
}

/// Ouvre le dossier d'archives d'une destination de miroir avec archive.
#[tauri::command]
async fn archive_reveal(app: AppHandle, target: String) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    off_ui(app, move |app, _| {
        let archive = Path::new(&target).join(filters::ARCHIVE_DIR);
        if !archive.is_dir() {
            return Err(ui_text("err.archive-none", &[]));
        }
        app.opener().open_path(archive.display().to_string(), None::<&str>).map_err(text)
    })
    .await
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckProgress {
    root: String,
    #[serde(flatten)]
    progress: IntegrityProgress,
}

#[tauri::command]
async fn integrity_start(app: AppHandle, state: State<'_, AppState>, root: String) -> CommandResult<IntegrityReport> {
    let control = Control::default();
    {
        let mut checks = state.checks.lock().unwrap();
        if checks.contains_key(&root) {
            return Err(ui_text("err.busy-check", &[]));
        }
        checks.insert(root.clone(), control.clone());
    }
    let store = state.store.clone();
    let reported = root.clone();
    let result = blocking(move || {
        let mut emit = |progress: &IntegrityProgress| {
            let _ = app.emit(
                "integrity:progress",
                CheckProgress {
                    root: reported.clone(),
                    progress: progress.clone(),
                },
            );
        };
        integrity::check(&store, Path::new(&reported), &control, &mut emit).map_err(text)
    })
    .await;
    state.checks.lock().unwrap().remove(&root);
    result
}

#[tauri::command]
fn integrity_cancel(state: State<'_, AppState>, root: String) {
    if let Some(control) = state.checks.lock().unwrap().get(&root) {
        control.cancel();
    }
}

#[tauri::command]
async fn integrity_baseline(app: AppHandle, root: String) -> CommandResult<Option<i64>> {
    off_ui(app, move |_, state| Ok(integrity::baseline(&state.store, Path::new(&root)).map(|m| m.created_at))).await
}

#[tauri::command]
async fn integrity_accept(app: AppHandle, root: String) -> CommandResult<()> {
    off_ui(app, move |_, state| integrity::accept(&state.store, Path::new(&root)).map_err(text)).await
}

#[tauri::command]
async fn integrity_export(app: AppHandle, root: String, target: String) -> CommandResult<usize> {
    off_ui(app, move |_, state| integrity::export_md5(&state.store, Path::new(&root), Path::new(&target)).map_err(text)).await
}

/// Lecture seule des données SMART ; le système demande lui-même l'autorisation d'administrateur.
#[tauri::command]
async fn health_scan(state: State<'_, AppState>) -> CommandResult<HealthReport> {
    let work = state.store.tmp_dir();
    let history = state.store.root().join("health");
    blocking(move || health::scan(&work, &history).map_err(text)).await
}

#[tauri::command]
async fn integrity_references(app: AppHandle) -> CommandResult<Vec<Reference>> {
    off_ui(app, |_, state| Ok(integrity::references(&state.store))).await
}

#[tauri::command]
async fn integrity_forget(app: AppHandle, root: String) -> CommandResult<()> {
    off_ui(app, move |_, state| integrity::forget(&state.store, Path::new(&root)).map_err(text)).await
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    id: String,
    #[serde(flatten)]
    progress: ToolProgress,
}

/// Exécute un outil sur son propre fil, avec progression et arrêt possible.
async fn job<T: Send + 'static>(
    app: AppHandle,
    state: &AppState,
    id: String,
    work: impl FnOnce(&Control, &mut dyn FnMut(&ToolProgress)) -> chud_core::Result<T> + Send + 'static,
) -> CommandResult<T> {
    let control = Control::default();
    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.contains_key(&id) {
            return Err(ui_text("err.busy-check", &[]));
        }
        jobs.insert(id.clone(), control.clone());
    }
    let reported = id.clone();
    let result = blocking(move || {
        let mut emit = |progress: &ToolProgress| {
            let _ = app.emit(
                "tool:progress",
                JobProgress {
                    id: reported.clone(),
                    progress: progress.clone(),
                },
            );
        };
        work(&control, &mut emit).map_err(text)
    })
    .await;
    state.jobs.lock().unwrap().remove(&id);
    result
}

#[tauri::command]
fn tool_cancel(state: State<'_, AppState>, id: String) {
    if let Some(control) = state.jobs.lock().unwrap().get(&id) {
        control.cancel();
    }
}

#[tauri::command]
async fn tool_compare(app: AppHandle, state: State<'_, AppState>, id: String, left: String, right: String, content: bool) -> CommandResult<CompareReport> {
    job(app, &state, id, move |control, emit| {
        tools::compare(Path::new(&left), Path::new(&right), content, control, emit)
    })
    .await
}

#[tauri::command]
async fn tool_duplicates(app: AppHandle, state: State<'_, AppState>, id: String, roots: Vec<String>, min_size: u64) -> CommandResult<DuplicateReport> {
    let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    job(app, &state, id, move |control, emit| tools::duplicates(&roots, min_size, control, emit)).await
}

#[tauri::command]
async fn tool_space(app: AppHandle, state: State<'_, AppState>, id: String, root: String) -> CommandResult<SpaceNode> {
    job(app, &state, id, move |control, emit| tools::space(Path::new(&root), control, emit)).await
}

#[tauri::command]
async fn tool_read(app: AppHandle, state: State<'_, AppState>, id: String, root: String) -> CommandResult<ReadReport> {
    job(app, &state, id, move |control, emit| tools::read_test(Path::new(&root), control, emit)).await
}

#[tauri::command]
async fn tool_cleanup(app: AppHandle, state: State<'_, AppState>, id: String, roots: Vec<String>, options: CleanupOptions) -> CommandResult<CleanupReport> {
    let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    job(app, &state, id, move |control, emit| cleanup::scan(&roots, &options, control, emit)).await
}

#[tauri::command]
async fn cleanup_places() -> CommandResult<Vec<Place>> {
    blocking(|| Ok(cleanup::places())).await
}

/// Corbeille pour les seuls fichiers cochés et confirmés par l'utilisateur.
#[tauri::command]
async fn cleanup_trash(app: AppHandle, items: Vec<CleanupItem>) -> CommandResult<TrashOutcome> {
    let owner = owner_window(&app);
    blocking(move || cleanup::trash_items(&items, owner).map_err(text)).await
}

#[tauri::command]
async fn tool_network(app: AppHandle, state: State<'_, AppState>, id: String, path: String, options: NetworkOptions) -> CommandResult<NetworkReport> {
    job(app, &state, id, move |control, emit| network::test(Path::new(&path), &options, control, emit)).await
}

/// Corbeille, après vérification que chaque copie a toujours le contenu trouvé par l'analyse.
#[tauri::command]
async fn duplicates_trash(app: AppHandle, group: DuplicateGroup, keep: String, remove: Vec<String>) -> CommandResult<Recycled> {
    let owner = owner_window(&app);
    blocking(move || tools::trash_duplicates(&group, &keep, &remove, owner).map_err(text)).await
}

#[tauri::command]
async fn archive_versions(target: String) -> CommandResult<Vec<ArchiveVersion>> {
    blocking(move || tools::archive_versions(Path::new(&target)).map_err(text)).await
}

#[tauri::command]
async fn archive_files(target: String, archive: String, stamp: String) -> CommandResult<Vec<ArchivedFile>> {
    blocking(move || tools::archive_files(Path::new(&target), &archive, &stamp).map_err(text)).await
}

#[tauri::command]
async fn archive_restore(target: String, archive: String, stamp: String, rels: Vec<String>, destination: String) -> CommandResult<Vec<String>> {
    blocking(move || tools::restore(Path::new(&target), &archive, &stamp, &rels, Path::new(&destination)).map_err(text)).await
}

#[tauri::command]
async fn log_read(app: AppHandle, id: String, search: String, offset: usize, limit: usize) -> CommandResult<LogPage> {
    off_ui(app, move |_, state| history::read_log(&state.store, &id, &search, offset, limit).map_err(text)).await
}

#[tauri::command]
async fn log_reveal(app: AppHandle, id: String) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    off_ui(app, move |app, state| {
        let path = history::log_path(&state.store, &id).map_err(text)?;
        app.opener().reveal_item_in_dir(path).map_err(text)
    })
    .await
}

#[tauri::command]
async fn data_reveal(app: AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    off_ui(app, |app, state| app.opener().open_path(state.store.root().display().to_string(), None::<&str>).map_err(text)).await
}

/// Ouvre un dossier dans l'explorateur, ou le dossier qui contient un fichier en le sélectionnant.
#[tauri::command]
async fn path_reveal(app: AppHandle, path: String) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    off_ui(app, move |app, _| {
        let target = Path::new(&path);
        if target.is_dir() {
            return app.opener().open_path(target.display().to_string(), None::<&str>).map_err(text);
        }
        if !target.exists() {
            return Err(ui_text("err.path-inaccessible", &[]));
        }
        app.opener().reveal_item_in_dir(target).map_err(text)
    })
    .await
}

/// Éteint l'ordinateur, demandé par l'interface au bout du compte à rebours qui suit la fin des
/// copies. Le cœur revérifie : rien ne s'éteint tant qu'une opération tourne ou attend son tour.
#[tauri::command]
async fn system_power_off(app: AppHandle) -> CommandResult<()> {
    off_ui(app, |_, state| {
        let waiting = state.queue.lock().unwrap().view().items.iter().any(|item| item.error.is_none());
        if run_is_active(state) || waiting {
            return Err(ui_text("err.power-busy", &[]));
        }
        chud_core::power::power_off().map_err(text)
    })
    .await
}

#[tauri::command]
fn releases_open(app: AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://github.com/BigFatDeuce/Chud-Copy/releases/latest", None::<&str>)
        .map_err(text)
}

#[tauri::command]
async fn settings_get(app: AppHandle) -> CommandResult<serde_json::Value> {
    off_ui(app, |_, state| Ok(state.store.load_settings())).await
}

#[tauri::command]
async fn settings_set(app: AppHandle, value: serde_json::Value) -> CommandResult<()> {
    off_ui(app, move |_, state| state.store.save_settings(&value).map_err(text)).await
}

/// Quitte après avoir arrêté proprement l'opération en cours.
#[tauri::command]
async fn app_quit(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state.queue.lock().unwrap().clear();
    if let Some(run) = state.run.lock().unwrap().as_ref() {
        run.control.cancel();
    }
    let deadline = Instant::now() + Duration::from_secs(12);
    while run_is_active(&state) && Instant::now() < deadline {
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(100)))
            .await
            .map_err(text)?;
    }
    app.exit(0);
    Ok(())
}

pub fn run() {
    #[cfg(target_os = "linux")]
    {
        // Le rendu DMA-BUF est le chemin rapide de WebKitGTK sur les pilotes Mesa (AMD, Intel).
        // Il n'est coupé que là où il affiche une fenêtre vide : pilote NVIDIA propriétaire, ou
        // machine sans accélération graphique (aucun nœud de rendu, comme une machine virtuelle).
        let nvidia = Path::new("/proc/driver/nvidia/version").exists();
        let render_node = std::fs::read_dir("/dev/dri")
            .map(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
            .unwrap_or(false);
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() && (nvidia || !render_node) {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        // Le pilote NVIDIA propriétaire reste la cause la plus fréquente d'échec EGL sous Wayland :
        // XWayland y est plus sûr. Un GDK_BACKEND choisi par l'utilisateur est toujours respecté.
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        if wayland && std::env::var_os("GDK_BACKEND").is_none() && nvidia {
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    let mut builder = tauri::Builder::default();
    // Une instance avec son propre dossier de données ne partage rien : elle peut coexister.
    if std::env::var_os("CHUD_COPY_DATA_DIR").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store = Store::open(store::default_root())?;
            let _ = history::mark_interrupted(&store);
            let engine = Rclone::new(engine_binary(), store.tmp_dir());
            app.manage(AppState {
                store,
                engine: Arc::new(engine),
                plans: Mutex::new(HashMap::new()),
                analyses: Mutex::new(HashMap::new()),
                run: Mutex::new(None),
                queue: Mutex::new(Queue::default()),
                checks: Mutex::new(HashMap::new()),
                jobs: Mutex::new(HashMap::new()),
                launching: Mutex::new(()),
            });
            // CHUD_COPY_BACKGROUND : la fenêtre s'ouvre hors de l'écran, hors de la barre des tâches et
            // sans prendre le focus. Les tests la pilotent ainsi sans gêner l'utilisateur de la machine.
            let background = std::env::var_os("CHUD_COPY_BACKGROUND").is_some();
            let mut config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .ok_or("fenêtre principale absente de la configuration")?;
            if background {
                config.center = false;
                config.x = Some(-4000.0);
                config.y = Some(-4000.0);
                config.focus = false;
                config.skip_taskbar = true;
            }
            // La barre native doit être demandée avant la création. Sous Wayland, une fenêtre créée
            // sans décorations reçoit une barre de titre vide, et `set_decorations(true)` appelé
            // ensuite n'y ajoute plus aucun bouton : ni fermer, ni réduire, ni agrandir.
            if NATIVE_CHROME {
                config.decorations = true;
            }
            let window = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
            // L'application est sombre : la barre native et les fenêtres de GTK le sont aussi. Le
            // thème se règle après la création, GTK ignorant celui demandé à la construction.
            if NATIVE_CHROME {
                let _ = window.set_theme(Some(tauri::Theme::Dark));
            }
            // Là où GTK dessine lui-même la barre (GNOME, Wayland sans décorations du serveur), il
            // suit ce réglage : réduire et agrandir y sont ajoutés s'ils manquent. Un bureau qui
            // dessine la barre à sa façon, comme KDE, l'ignore et garde ses propres boutons.
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::GtkSettingsExt;
                if let Some(settings) = gtk::Settings::default() {
                    let current = settings.gtk_decoration_layout().map(|layout| layout.to_string()).unwrap_or_default();
                    let wanted = complete_window_buttons(&current);
                    if wanted != current {
                        settings.set_gtk_decoration_layout(Some(&wanted));
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            let _ = window;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if run_is_active(&state) {
                    api.prevent_close();
                    let _ = window.emit("app:close-blocked", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            play_sound,
            tasks_list,
            task_save,
            task_delete,
            task_duplicate,
            tasks_export,
            tasks_import,
            task_validate,
            disks_list,
            plan_start,
            plan_cancel,
            plan_forget,
            releases_open,
            plan_query,
            run_start,
            run_pause,
            run_resume,
            run_cancel,
            run_current,
            queue_get,
            queue_add,
            queue_remove,
            queue_shift,
            queue_clear,
            queue_resume,
            history_list,
            history_get,
            history_delete,
            history_clear,
            tasks_last_runs,
            junk_list,
            archive_reveal,
            integrity_start,
            integrity_cancel,
            integrity_baseline,
            integrity_accept,
            integrity_export,
            health_scan,
            integrity_references,
            integrity_forget,
            tool_cancel,
            tool_compare,
            tool_duplicates,
            tool_space,
            tool_read,
            tool_cleanup,
            cleanup_places,
            cleanup_trash,
            tool_network,
            drives_status,
            drive_accept,
            duplicates_trash,
            archive_versions,
            archive_files,
            archive_restore,
            log_read,
            log_reveal,
            data_reveal,
            path_reveal,
            system_power_off,
            settings_get,
            settings_set,
            app_quit,
        ])
        .run(tauri::generate_context!())
        .expect("impossible de démarrer Chud Copy");
}

#[cfg(test)]
mod tests {
    use super::complete_window_buttons;

    #[test]
    fn window_buttons_are_always_complete() {
        // GNOME : seulement « fermer ».
        assert_eq!(complete_window_buttons("appmenu:close"), "appmenu:minimize,maximize,close");
        assert_eq!(complete_window_buttons("menu:close"), "menu:minimize,maximize,close");
        // Rien de réglé, ou aucun bouton.
        assert_eq!(complete_window_buttons(""), ":minimize,maximize,close");
        assert_eq!(complete_window_buttons("icon:"), "icon:minimize,maximize,close");
        // Un bureau déjà complet n'est pas touché, à droite comme à gauche.
        assert_eq!(complete_window_buttons("icon:minimize,maximize,close"), "icon:minimize,maximize,close");
        assert_eq!(complete_window_buttons("close,minimize,maximize:"), "close,minimize,maximize:");
        // Boutons à gauche : les manquants suivent « fermer » du même côté.
        assert_eq!(complete_window_buttons("close:menu"), "close,minimize,maximize:menu");
        // Un seul bouton manquant est ajouté, « fermer » reste au bord.
        assert_eq!(complete_window_buttons(":minimize,close"), ":minimize,maximize,close");
    }
}
