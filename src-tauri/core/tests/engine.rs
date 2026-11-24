//! Tests de bout en bout contre le vrai binaire rclone embarqué.
//!
//! Chaque test travaille uniquement dans un dossier temporaire créé pour lui.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chud_core::engine::rclone::Rclone;
use chud_core::engine::{Analysis, Control, Engine, EngineEvent, EngineStats, EntryKind, JobSpec, TransferOutcome, VerifyOutcome};
use chud_core::history::{self, LinkPhase, RunRecord, RunStatus};
use chud_core::model::{Endpoint, Filters, Link, Mode, Options, Symlinks, Task};
use chud_core::plan::{self, Plan, PlanQuery};
use chud_core::runner::{self, RunHandle};
use chud_core::store::Store;
use chud_core::validate::Severity;

fn rclone_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "rclone-x86_64-pc-windows-msvc.exe"
    } else {
        "rclone-x86_64-unknown-linux-gnu"
    };
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../binaries").join(name)
}

struct Lab {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Store,
    engine: Arc<dyn Engine>,
}

impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let store = Store::open(root.join("data")).unwrap();
        let engine: Arc<dyn Engine> = Arc::new(Rclone::new(rclone_binary(), store.tmp_dir()));
        Self {
            _dir: dir,
            root,
            store,
            engine,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn task(&self, mode: Mode, pairs: &[(&str, &str, &str)]) -> Task {
        let mut task = Task {
            name: "Essai".into(),
            mode,
            ..Task::default()
        };
        for (i, (source, destination, subfolder)) in pairs.iter().enumerate() {
            task.sources.push(Endpoint {
                id: format!("s{i}"),
                path: self.path(source).display().to_string(),
                ..Default::default()
            });
            task.destinations.push(Endpoint {
                id: format!("d{i}"),
                path: self.path(destination).display().to_string(),
                ..Default::default()
            });
            task.links.push(Link {
                id: format!("l{i}"),
                source_id: format!("s{i}"),
                destination_id: format!("d{i}"),
                subfolder: subfolder.to_string(),
                enabled: true,
            });
        }
        self.store.save_task(task).unwrap()
    }

    fn analyze(&self, task: &Task) -> Arc<Plan> {
        Arc::new(plan::analyze_task(self.engine.as_ref(), task, &Control::default(), &|_| {}).unwrap())
    }

    fn start(&self, task: &Task, plan: Option<Arc<Plan>>) -> RunHandle {
        runner::start(self.engine.clone(), self.store.clone(), task.clone(), plan, Arc::new(|_: &RunRecord| {})).unwrap()
    }

    fn run(&self, task: &Task, plan: Option<Arc<Plan>>) -> RunRecord {
        let handle = self.start(task, plan);
        wait(&handle);
        history::get(&self.store, &handle.run_id).unwrap()
    }
}

fn wait(handle: &RunHandle) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "opération trop longue");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Image complète d'un dossier : chemin, contenu et date.
fn snapshot(root: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
    let mut out = BTreeMap::new();
    for entry in walkdir(root) {
        let meta = fs::metadata(&entry).unwrap();
        let rel = entry.strip_prefix(root).unwrap().display().to_string().replace('\\', "/");
        out.insert(rel, (fs::read(&entry).unwrap(), meta.modified().unwrap()));
    }
    out
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                out.push(path);
            }
        }
    }
    out
}

fn names(root: &Path) -> Vec<String> {
    snapshot(root).into_keys().collect()
}

fn kinds(plan: &Plan, kind: EntryKind) -> Vec<String> {
    let page = plan::query(
        plan,
        &PlanQuery {
            kinds: vec![kind],
            limit: 500,
            ..PlanQuery::default()
        },
    );
    let mut out: Vec<String> = page.rows.into_iter().map(|r| r.rel).collect();
    out.sort();
    out
}

fn fixture(lab: &Lab) {
    lab.write("src/same.txt", "identique");
    lab.write("src/new.txt", "nouveau");
    lab.write("src/sub/deep/été 日本語 🚀.txt", "unicode");
    lab.write("src/sub/changed.txt", "version source longue");
    lab.write("dst/sub/changed.txt", "vieux");
    lab.write("dst/extra/orphan.txt", "orphelin");
    lab.write("dst/gone.txt", "à supprimer");
    // Une même date rend `same.txt` réellement identique des deux côtés.
    lab.write("dst/same.txt", "identique");
    let when = fs::metadata(lab.path("src/same.txt")).unwrap().modified().unwrap();
    fs::File::options()
        .write(true)
        .open(lab.path("dst/same.txt"))
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn engine_reports_its_version() {
    let lab = Lab::new();
    assert!(lab.engine.version().unwrap().starts_with("rclone v"));
}

#[test]
fn analysis_changes_nothing_and_predicts_the_run() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let before_src = snapshot(&lab.path("src"));
    let before_dst = snapshot(&lab.path("dst"));

    let plan = lab.analyze(&task);

    assert_eq!(snapshot(&lab.path("src")), before_src, "l'analyse a modifié la source");
    assert_eq!(snapshot(&lab.path("dst")), before_dst, "l'analyse a modifié la destination");
    assert_eq!(kinds(&plan, EntryKind::Copy), vec!["new.txt", "sub/deep/été 日本語 🚀.txt"]);
    assert_eq!(kinds(&plan, EntryKind::Update), vec!["sub/changed.txt"]);
    assert_eq!(kinds(&plan, EntryKind::Same), vec!["same.txt"]);
    assert_eq!(kinds(&plan, EntryKind::Delete), vec!["extra/orphan.txt", "gone.txt"]);
    assert_eq!(plan.totals.copy_bytes, "nouveau".len() as u64 + "unicode".len() as u64);
    assert!(plan.totals.delete_bytes > 0);

    let record = lab.run(&task, Some(plan));
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(names(&lab.path("dst")), names(&lab.path("src")));
}

#[test]
fn the_source_is_never_touched_in_any_mode() {
    for mode in [Mode::Update, Mode::Full, Mode::AddOnly, Mode::Mirror, Mode::MirrorArchive] {
        let lab = Lab::new();
        fixture(&lab);
        let task = lab.task(mode, &[("src", "dst", "")]);
        let before = snapshot(&lab.path("src"));
        let plan = mode.removes_from_destination().then(|| lab.analyze(&task));
        let record = lab.run(&task, plan);
        assert_eq!(record.status, RunStatus::Completed, "{mode:?} {:?}", record.errors);
        assert_eq!(snapshot(&lab.path("src")), before, "{mode:?} a modifié la source");
    }
}

#[test]
fn a_date_rounded_by_a_cloud_service_does_not_force_a_full_read() {
    let lab = Lab::new();
    // Même taille, contenu différent : seule une relecture complète verrait la différence.
    lab.write("src/f.txt", "aaaa");
    lab.write("dst/f.txt", "bbbb");
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let touch = |rel: &str, at: SystemTime| fs::File::options().write(true).open(lab.path(rel)).unwrap().set_modified(at).unwrap();
    touch("src/f.txt", when);

    touch("dst/f.txt", when + Duration::from_millis(400));
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    assert_eq!(kinds(&lab.analyze(&task), EntryKind::Same), vec!["f.txt"]);

    // Au-delà d'une seconde, la vérification du contenu reste en place.
    touch("dst/f.txt", when + Duration::from_secs(5));
    assert_eq!(kinds(&lab.analyze(&task), EntryKind::Update), vec!["f.txt"]);
}

#[test]
fn update_mode_keeps_extra_files() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert_eq!(kinds(&plan, EntryKind::Delete), Vec::<String>::new());
    assert_eq!(kinds(&plan, EntryKind::Extra), vec!["extra/orphan.txt", "gone.txt"]);

    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(fs::read_to_string(lab.path("dst/sub/changed.txt")).unwrap(), "version source longue");
    assert!(lab.path("dst/gone.txt").exists());
    assert!(lab.path("dst/extra/orphan.txt").exists());
    assert_eq!(fs::read_to_string(lab.path("dst/sub/deep/été 日本語 🚀.txt")).unwrap(), "unicode");
}

#[test]
fn add_only_never_replaces() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::AddOnly, &[("src", "dst", "")]);
    assert_eq!(lab.analyze(&task).totals.update, 0);
    lab.run(&task, None);
    assert_eq!(fs::read_to_string(lab.path("dst/sub/changed.txt")).unwrap(), "vieux");
    assert!(lab.path("dst/new.txt").exists());
}

#[test]
fn full_copy_rewrites_identical_files() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Full, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert_eq!(kinds(&plan, EntryKind::Same), Vec::<String>::new());
    assert!(kinds(&plan, EntryKind::Update).contains(&"same.txt".to_string()));
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(record.totals.transfers, 4);
}

#[test]
fn mirror_requires_a_validated_analysis() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let denied = runner::start(lab.engine.clone(), lab.store.clone(), task.clone(), None, Arc::new(|_: &RunRecord| {}));
    assert!(denied.is_err());
    assert!(lab.path("dst/gone.txt").exists());

    // Une tâche modifiée après l'analyse invalide celle-ci.
    let plan = lab.analyze(&task);
    let edited = lab.store.save_task(task).unwrap();
    let denied = runner::start(lab.engine.clone(), lab.store.clone(), edited, Some(plan), Arc::new(|_: &RunRecord| {}));
    assert!(denied.is_err());
    assert!(lab.path("dst/gone.txt").exists());
}

#[test]
fn mirror_cannot_delete_more_than_what_was_validated() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert_eq!(plan.totals.delete, 2);

    // Un fichier de plus apparaît en destination entre l'analyse et l'exécution.
    lab.write("dst/arrive-apres-analyse.txt", "ne doit pas disparaitre sans accord");
    let record = lab.run(&task, Some(plan));

    assert_ne!(record.status, RunStatus::Completed);
    assert!(
        record.errors.iter().any(|e| e.error.as_ref().is_some_and(|f| f.category == "delete-limit")),
        "{:?}",
        record.errors
    );
    let remaining = names(&lab.path("dst"));
    assert!(remaining.contains(&"arrive-apres-analyse.txt".to_string()));
    assert!(remaining.contains(&"gone.txt".to_string()), "rien ne doit bouger sur cette base");
    assert!(!remaining.contains(&"new.txt".to_string()), "aucune copie ne doit commencer");
}

/// Un moteur à qui un fichier échappe sans message, comme rclone face aux fichiers cloud de Windows.
struct Blind(Arc<dyn Engine>);

impl Engine for Blind {
    fn name(&self) -> String {
        self.0.name()
    }

    fn version(&self) -> chud_core::Result<String> {
        self.0.version()
    }

    fn analyze(&self, job: &JobSpec, control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> chud_core::Result<Analysis> {
        let mut analysis = self.0.analyze(job, control, progress)?;
        analysis.entries.retain(|e| e.rel != "cloud.txt" || e.kind == EntryKind::Delete);
        for entry in &mut analysis.entries {
            if entry.rel == "cloud.txt" {
                entry.kind = EntryKind::Extra;
            }
        }
        Ok(analysis)
    }

    fn transfer(&self, job: &JobSpec, control: &Control, events: &mut dyn FnMut(EngineEvent)) -> chud_core::Result<TransferOutcome> {
        self.0.transfer(job, control, events)
    }

    fn verify(&self, job: &JobSpec, files: &[String], control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> chud_core::Result<VerifyOutcome> {
        self.0.verify(job, files, control, progress)
    }
}

#[test]
fn files_the_engine_cannot_see_are_reported_and_block_a_mirror() {
    let lab = Lab::new();
    lab.write("src/normal.txt", "x");
    lab.write("src/cloud.txt", "x");
    lab.write("dst/cloud.txt", "x");
    let blind: Arc<dyn Engine> = Arc::new(Blind(lab.engine.clone()));

    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let plan = plan::analyze_task(blind.as_ref(), &task, &Control::default(), &|_| {}).unwrap();
    assert_eq!(plan.links[0].unseen_files, 1);
    let warning = plan.issues.iter().find(|i| i.code == "unseen-files").expect("écart non signalé");
    assert_eq!(warning.severity, Severity::Warning);

    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = Arc::new(plan::analyze_task(blind.as_ref(), &task, &Control::default(), &|_| {}).unwrap());
    assert!(plan.issues.iter().any(|i| i.code == "unseen-files-mirror" && i.severity == Severity::Error));
    let refused = runner::start(blind, lab.store.clone(), task, Some(plan), Arc::new(|_: &RunRecord| {}));
    assert!(refused.is_err());
    assert!(lab.path("dst/cloud.txt").exists());
}

#[test]
fn a_complete_view_of_the_source_raises_no_alarm() {
    let lab = Lab::new();
    fixture(&lab);
    lab.write("src/.cache/x.txt", "x");
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.filters.exclude_extensions = vec!["jpg".into()];
    task.options.include_hidden = false;
    let task = lab.store.save_task(task).unwrap();
    let plan = lab.analyze(&task);
    assert_eq!(plan.links[0].unseen_files, 0);
}

#[test]
fn a_file_replaced_by_another_with_same_count_is_still_caught() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    // Même nombre de suppressions qu'à l'analyse, mais plus les mêmes fichiers.
    fs::remove_file(lab.path("dst/gone.txt")).unwrap();
    lab.write("dst/autre.txt", "jamais validé");
    let record = lab.run(&task, Some(plan));
    assert_ne!(record.status, RunStatus::Completed);
    assert!(lab.path("dst/autre.txt").exists());
}

#[test]
fn fewer_deletions_than_validated_is_fine() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    fs::remove_file(lab.path("dst/gone.txt")).unwrap();
    let record = lab.run(&task, Some(plan));
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(names(&lab.path("dst")), names(&lab.path("src")));
}

#[test]
fn mirror_with_archive_moves_instead_of_deleting() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::MirrorArchive, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert_eq!(kinds(&plan, EntryKind::Delete), vec!["extra/orphan.txt", "gone.txt"]);

    let record = lab.run(&task, Some(plan));
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert!(!lab.path("dst/gone.txt").exists());

    let archived = names(&lab.path("dst/_Archives Chud Copy"));
    assert!(archived.iter().any(|p| p.ends_with("/gone.txt")), "{archived:?}");
    assert!(archived.iter().any(|p| p.ends_with("/extra/orphan.txt")));
    assert!(
        archived.iter().any(|p| p.ends_with("/sub/changed.txt")),
        "l'ancienne version doit être archivée"
    );

    // Une seconde passe ne doit ni vider ni archiver l'archive elle-même.
    let task = lab.store.save_task(task).unwrap();
    let plan = lab.analyze(&task);
    assert_eq!(plan.totals.delete, 0);
    lab.run(&task, Some(plan));
    assert_eq!(names(&lab.path("dst/_Archives Chud Copy")), archived);
}

#[test]
fn archives_left_by_earlier_backups_are_never_deleted() {
    let lab = Lab::new();
    lab.write("src/a.txt", "a");
    lab.write("src/Photos/p.jpg", "p");
    lab.write("dst/a.txt", "a");
    lab.write("dst/.chudcopy-archive/2026-01-01/ancien.txt", "racine");
    lab.write("dst/Photos/.chudcopy-archive/2026-02-01/vieille.jpg", "sous-dossier");
    lab.write("dst/_Archives Chud Copy/2026-03-01 10h00 00/nouvelle.txt", "visible");
    lab.write("dst/Photos/_Archives Chud Copy/2026-04-01 10h00 00/autre.jpg", "visible");
    lab.write("dst/en-trop.txt", "x");

    for mode in [Mode::Mirror, Mode::MirrorArchive] {
        let task = lab.task(mode, &[("src", "dst", "")]);
        let plan = lab.analyze(&task);
        assert_eq!(kinds(&plan, EntryKind::Delete), vec!["en-trop.txt"], "{mode:?}");
    }

    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    let record = lab.run(&task, Some(plan));
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert!(lab.path("dst/.chudcopy-archive/2026-01-01/ancien.txt").exists());
    assert!(lab.path("dst/Photos/.chudcopy-archive/2026-02-01/vieille.jpg").exists());
    assert!(lab.path("dst/_Archives Chud Copy/2026-03-01 10h00 00/nouvelle.txt").exists());
    assert!(lab.path("dst/Photos/_Archives Chud Copy/2026-04-01 10h00 00/autre.jpg").exists());
    assert!(!lab.path("dst/en-trop.txt").exists());
}

#[test]
fn an_empty_source_is_flagged_before_deleting_everything() {
    let lab = Lab::new();
    lab.write("dst/precieux.txt", "contenu");
    fs::create_dir_all(lab.path("src")).unwrap();
    let task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert!(plan.links[0].source_looks_empty);
    assert!(plan.issues.iter().any(|i| i.code == "empty-source"));
    assert!(lab.path("dst/precieux.txt").exists());
}

#[test]
fn a_missing_source_deletes_nothing_and_other_links_continue() {
    let lab = Lab::new();
    lab.write("bon/fichier.txt", "ok");
    lab.write("dst1/garde.txt", "garde");
    fs::create_dir_all(lab.path("dst2")).unwrap();
    let task = lab.task(Mode::Mirror, &[("absent", "dst1", ""), ("bon", "dst2", "")]);
    let plan = lab.analyze(&task);
    assert!(!plan.links[0].analyzed);

    let record = lab.run(&task, Some(plan));
    assert_eq!(record.status, RunStatus::CompletedWithErrors);
    assert_eq!(record.links[0].phase, LinkPhase::Skipped);
    assert_eq!(record.links[1].phase, LinkPhase::Done);
    assert!(lab.path("dst1/garde.txt").exists(), "une source absente ne doit rien supprimer");
    assert!(lab.path("dst2/fichier.txt").exists());
}

#[test]
fn one_source_to_several_destinations_and_shared_root_with_subfolders() {
    let lab = Lab::new();
    lab.write("a/photo.txt", "a");
    lab.write("b/photo.txt", "b");
    let task = lab.task(
        Mode::Update,
        &[("a", "backup1", ""), ("a", "backup2", ""), ("a", "stock", "A"), ("b", "stock", "B")],
    );
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(fs::read_to_string(lab.path("backup1/photo.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(lab.path("backup2/photo.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(lab.path("stock/A/photo.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(lab.path("stock/B/photo.txt")).unwrap(), "b");
}

#[test]
fn merged_sources_show_their_collisions_in_the_analysis() {
    let lab = Lab::new();
    lab.write("a/commun.txt", "a");
    lab.write("b/commun.txt", "b");
    lab.write("b/seul.txt", "b");
    let mut task = lab.task(Mode::Update, &[("a", "stock", ""), ("b", "stock", "")]);
    task.options.allow_merge = true;
    let task = lab.store.save_task(task).unwrap();
    let plan = lab.analyze(&task);
    assert_eq!(kinds(&plan, EntryKind::Conflict), vec!["commun.txt"]);
}

#[test]
fn filters_apply_and_ignored_files_are_listed() {
    let lab = Lab::new();
    lab.write("src/photo.JPG", "12345");
    lab.write("src/image.jpg", "12345");
    lab.write("src/notes.txt", "12345");
    lab.write("src/brouillon.jpg", "12345");
    lab.write("src/cache/temp.jpg", "12345");
    lab.write("src/gros.jpg", &"x".repeat(5000));
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.filters = Filters {
        include_extensions: vec!["jpg".into()],
        exclude_files: vec!["brouillon*".into()],
        exclude_dirs: vec!["cache".into()],
        max_size: Some(1000),
        ..Filters::default()
    };
    let task = lab.store.save_task(task).unwrap();

    let plan = lab.analyze(&task);
    assert_eq!(kinds(&plan, EntryKind::Copy), vec!["image.jpg", "photo.JPG"]);
    assert_eq!(
        kinds(&plan, EntryKind::Ignored),
        vec!["brouillon.jpg", "cache/temp.jpg", "gros.jpg", "notes.txt"]
    );

    lab.run(&task, None);
    assert_eq!(names(&lab.path("dst")), vec!["image.jpg", "photo.JPG"]);
}

#[test]
fn useless_files_are_skipped_only_when_asked() {
    let junk = [
        "NTUSER.DAT",
        "ntuser.dat.LOG1",
        "NTUSER.DAT{016888bd}.TM.blf",
        "NTUSER.INI",
        "UsrClass.dat",
        "UsrClass.dat.LOG2",
        "sous/desktop.ini",
        "sous/Thumbs.db",
        "sous/IconCache.db",
        "sous/thumbcache_1024.db",
        "travail.TMP",
        "travail.temp",
        "~$rapport.docx",
        "ancien.BAK",
        "Raccourci.lnk",
        "verrou.lock",
        "pagefile.sys",
        "hiberfil.sys",
        "sous/.DS_Store",
        "sous/._photo.jpg",
        "sous/.directory",
        "sous/.fuse_hidden0001",
        "sous/.~lock.compta.ods#",
        "sous/rapport.txt~",
        "sous/.swap.swp",
        "crash.DMP",
        "film.mkv.crdownload",
        "iso.part",
        "torrent.!qB",
        "gros.aria2",
        ".debris/2026/ancien.docx",
        ".tmp.driveupload/bloc",
        ".nextcloudsync.log",
        "._sync_ab12.db",
    ];
    let kept = [
        "rapport.docx",
        "sous/lockfile.txt",
        "sous/montmp.txt",
        "sous/backup.zip",
        "archive.part1.rar",
        "photo~1.jpg",
        ".gitignore",
        ".megaignore",
        "journal.log",
        "ancien.old",
        "module.pyc",
        "__pycache__/cache.pyc",
    ];

    let lab = Lab::new();
    for name in junk.iter().chain(kept.iter()) {
        lab.write(&format!("src/{name}"), "x");
    }

    let task = lab.task(Mode::Update, &[("src", "normal", "")]);
    assert_eq!(
        lab.analyze(&task).totals.copy,
        (junk.len() + kept.len()) as u64,
        "décochée, l'option ne retire rien"
    );

    let mut task = lab.task(Mode::Update, &[("src", "propre", "")]);
    task.filters.exclude_junk = true;
    let task = lab.store.save_task(task).unwrap();
    let plan = lab.analyze(&task);
    assert_eq!(plan.totals.ignored, junk.len() as u64);
    assert_eq!(plan.links[0].unseen_files, 0);
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    let mut expected: Vec<String> = kept.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(names(&lab.path("propre")), expected);
}

#[test]
fn date_filters_select_by_age() {
    let lab = Lab::new();
    lab.write("src/ancien.txt", "a");
    lab.write("src/recent.txt", "r");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(lab.path("src/ancien.txt"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.filters.newer_than = Some("2020-01-01".into());
    let task = lab.store.save_task(task).unwrap();
    assert_eq!(kinds(&lab.analyze(&task), EntryKind::Copy), vec!["recent.txt"]);
}

#[test]
fn excluded_files_survive_a_mirror() {
    let lab = Lab::new();
    lab.write("src/garde.txt", "x");
    lab.write("dst/notes.tmp", "exclu, donc intouchable");
    let mut task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    task.filters.exclude_extensions = vec!["tmp".into()];
    let task = lab.store.save_task(task).unwrap();
    let plan = lab.analyze(&task);
    assert_eq!(plan.totals.delete, 0);
    lab.run(&task, Some(plan));
    assert!(lab.path("dst/notes.tmp").exists());
}

#[test]
fn dotfiles_follow_the_hidden_option() {
    let lab = Lab::new();
    lab.write("src/.cache/x.txt", "x");
    lab.write("src/.env", "x");
    lab.write("src/visible.txt", "x");
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.include_hidden = false;
    let task = lab.store.save_task(task).unwrap();
    lab.run(&task, None);
    assert_eq!(names(&lab.path("dst")), vec!["visible.txt"]);
}

#[cfg(windows)]
#[test]
fn windows_hidden_attribute_is_honoured() {
    let lab = Lab::new();
    lab.write("src/secret [1].txt", "x");
    lab.write("src/visible.txt", "x");
    lab.write("src/dossier/dedans.txt", "x");
    for name in ["src/secret [1].txt", "src/dossier"] {
        let status = std::process::Command::new("attrib").arg("+h").arg(lab.path(name)).status().unwrap();
        assert!(status.success());
    }
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.include_hidden = false;
    let task = lab.store.save_task(task).unwrap();
    lab.run(&task, None);
    assert_eq!(names(&lab.path("dst")), vec!["visible.txt"]);
}

#[test]
fn empty_folders_follow_their_option() {
    let lab = Lab::new();
    lab.write("src/a.txt", "x");
    fs::create_dir_all(lab.path("src/vide")).unwrap();
    let mut task = lab.task(Mode::Update, &[("src", "avec", "")]);
    lab.run(&task, None);
    assert!(lab.path("avec/vide").is_dir());
    task = lab.task(Mode::Update, &[("src", "sans", "")]);
    task.options.empty_dirs = false;
    let task = lab.store.save_task(task).unwrap();
    lab.run(&task, None);
    assert!(!lab.path("sans/vide").exists());
}

#[test]
fn dates_are_preserved() {
    let lab = Lab::new();
    lab.write("src/date.txt", "x");
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    fs::File::options()
        .write(true)
        .open(lab.path("src/date.txt"))
        .unwrap()
        .set_modified(when)
        .unwrap();
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    lab.run(&task, None);
    assert_eq!(fs::metadata(lab.path("dst/date.txt")).unwrap().modified().unwrap(), when);
}

#[test]
fn skip_newer_keeps_a_more_recent_destination() {
    let lab = Lab::new();
    lab.write("src/f.txt", "ancienne source");
    lab.write("dst/f.txt", "destination plus récente");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(lab.path("src/f.txt")).unwrap().set_modified(old).unwrap();
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.skip_newer = true;
    let task = lab.store.save_task(task).unwrap();
    assert_eq!(lab.analyze(&task).totals.update, 0);
    lab.run(&task, None);
    assert_eq!(fs::read_to_string(lab.path("dst/f.txt")).unwrap(), "destination plus récente");
}

#[test]
fn long_paths_and_unicode_are_copied() {
    let lab = Lab::new();
    let mut rel = String::from("src");
    for i in 0..14 {
        rel.push_str(&format!("/dossier-très-long-numéro-{i:02}-αβγ-日本語-🚀-remplissage"));
    }
    rel.push_str("/fichier final.txt");
    assert!(lab.path(&rel).display().to_string().chars().count() > 300);
    lab.write(&rel, "bout du chemin");
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(record.totals.transfers, 1);
    let copied = rel.replacen("src", "dst", 1);
    assert_eq!(fs::read_to_string(lab.path(&copied)).unwrap(), "bout du chemin");
}

#[test]
fn a_verified_run_confirms_every_written_file() {
    let lab = Lab::new();
    fixture(&lab);
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.verify_after = true;
    let task = lab.store.save_task(task).unwrap();
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(record.links[0].verify_checked, 3);
    assert_eq!(record.links[0].verify_failures, 0);
}

#[test]
fn verification_catches_a_corrupted_file() {
    let lab = Lab::new();
    lab.write("src/f.txt", "contenu d'origine");
    lab.write("dst/f.txt", "contenu altéré!!!");
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let job = plan::job_for(&task, &task.resolved_links()[0], None, "x");
    let outcome = lab.engine.verify(&job, &["f.txt".to_string()], &Control::default(), &mut |_| {}).unwrap();
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(outcome.failures[0].category, "corrupt");
}

#[cfg(unix)]
#[test]
fn symlinks_follow_their_option() {
    let lab = Lab::new();
    lab.write("src/vrai.txt", "cible");
    std::os::unix::fs::symlink("vrai.txt", lab.path("src/lien.txt")).unwrap();

    let task = lab.task(Mode::Update, &[("src", "ignore", "")]);
    lab.run(&task, None);
    assert!(fs::symlink_metadata(lab.path("ignore/lien.txt")).is_err());

    let mut task = lab.task(Mode::Update, &[("src", "suivi", "")]);
    task.options.symlinks = Symlinks::Follow;
    let task = lab.store.save_task(task).unwrap();
    lab.run(&task, None);
    assert!(fs::symlink_metadata(lab.path("suivi/lien.txt")).unwrap().file_type().is_file());

    let mut task = lab.task(Mode::Update, &[("src", "garde", "")]);
    task.options.symlinks = Symlinks::Preserve;
    let task = lab.store.save_task(task).unwrap();
    lab.run(&task, None);
    assert!(fs::symlink_metadata(lab.path("garde/lien.txt")).unwrap().file_type().is_symlink());
}

fn big_file(lab: &Lab, rel: &str, megabytes: usize) {
    let path = lab.path(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut block = vec![0u8; 1024 * 1024];
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for chunk in block.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
    let mut file = fs::File::create(path).unwrap();
    for _ in 0..megabytes {
        std::io::Write::write_all(&mut file, &block).unwrap();
    }
}

#[test]
fn cancelling_leaves_no_partial_file() {
    let lab = Lab::new();
    big_file(&lab, "src/gros.bin", 1500);
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let handle = lab.start(&task, None);

    let deadline = Instant::now() + Duration::from_secs(20);
    while handle.snapshot().links[0].bytes == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.control.cancel();
    wait(&handle);

    let record = history::get(&lab.store, &handle.run_id).unwrap();
    assert_eq!(record.status, RunStatus::Cancelled);
    let leftovers = names(&lab.path("dst"));
    assert!(!leftovers.iter().any(|n| n.contains("partial")), "{leftovers:?}");
    // L'annulation peut arriver juste après la fin du fichier : il est alors complet, jamais tronqué.
    if let Ok(meta) = fs::metadata(lab.path("dst/gros.bin")) {
        assert_eq!(meta.len(), 1500 * 1024 * 1024, "un fichier incomplet ne doit jamais paraître terminé");
    }
}

#[test]
fn pause_really_stops_the_engine_and_resume_completes() {
    let lab = Lab::new();
    big_file(&lab, "src/gros.bin", 1500);
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let handle = lab.start(&task, None);

    let deadline = Instant::now() + Duration::from_secs(20);
    while handle.snapshot().links[0].bytes == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.control.pause();
    std::thread::sleep(Duration::from_millis(1400));
    let first = handle.snapshot().links[0].bytes;
    std::thread::sleep(Duration::from_millis(1600));
    let second = handle.snapshot().links[0].bytes;
    if !handle.is_finished() {
        assert_eq!(first, second, "le moteur a continué pendant la pause");
    }

    handle.control.resume();
    wait(&handle);
    let record = history::get(&lab.store, &handle.run_id).unwrap();
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(fs::metadata(lab.path("dst/gros.bin")).unwrap().len(), 1500 * 1024 * 1024);
}

#[test]
fn progress_comes_from_the_engine() {
    let lab = Lab::new();
    big_file(&lab, "src/gros.bin", 800);
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let seen: Arc<Mutex<Vec<(u64, u64, f64)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handle = runner::start(
        lab.engine.clone(),
        lab.store.clone(),
        task,
        None,
        Arc::new(move |r: &RunRecord| sink.lock().unwrap().push((r.totals.bytes, r.totals.total_bytes, r.totals.speed))),
    )
    .unwrap();
    wait(&handle);
    let seen = seen.lock().unwrap();
    let last = seen.last().unwrap();
    assert_eq!(last.0, 800 * 1024 * 1024);
    assert_eq!(last.1, 800 * 1024 * 1024);
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "la progression ne doit jamais reculer");
}

#[test]
fn a_run_is_recorded_with_its_journal() {
    let lab = Lab::new();
    fixture(&lab);
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let record = lab.run(&task, None);
    let listed = history::list(&lab.store).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, record.id);
    let log = history::read_log(&lab.store, &record.id, "copied", 0, 50).unwrap();
    assert!(log.total >= 3, "{:?}", log.lines);
}

#[cfg(unix)]
#[test]
fn permissions_are_kept_and_never_block_a_copy() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new();
    lab.write("src/prive.sh", "#!/bin/sh");
    fs::set_permissions(lab.path("src/prive.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(fs::metadata(lab.path("dst/prive.sh")).unwrap().permissions().mode() & 0o777, 0o750);
}

/// Un fichier appartenant à quelqu'un d'autre, préparé par le script de test Linux.
#[cfg(unix)]
#[test]
fn a_file_owned_by_someone_else_is_still_copied() {
    let Some(foreign) = std::env::var_os("CHUD_FOREIGN_DIR") else { return };
    let lab = Lab::new();
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    fs::create_dir_all(lab.path("src")).unwrap();
    task.sources[0].path = PathBuf::from(foreign).display().to_string();
    let task = lab.store.save_task(task).unwrap();
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert!(lab.path("dst/autre-proprietaire.txt").exists());
}

#[cfg(unix)]
fn running_as_root() -> bool {
    fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0).unwrap_or(true)
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_reported_and_the_rest_is_copied() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let lab = Lab::new();
    lab.write("src/interdit.txt", "secret");
    lab.write("src/libre.txt", "libre");
    fs::set_permissions(lab.path("src/interdit.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.retries = 1;
    let task = lab.store.save_task(task).unwrap();
    let record = lab.run(&task, None);
    fs::set_permissions(lab.path("src/interdit.txt"), fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(record.status, RunStatus::CompletedWithErrors);
    assert!(lab.path("dst/libre.txt").exists());
    assert!(
        record.errors.iter().any(|e| e.error.as_ref().is_some_and(|f| f.category == "denied")),
        "{:?}",
        record.errors
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_destination_is_refused_before_any_copy() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let lab = Lab::new();
    lab.write("src/a.txt", "x");
    fs::create_dir_all(lab.path("dst")).unwrap();
    fs::set_permissions(lab.path("dst"), fs::Permissions::from_mode(0o555)).unwrap();
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let report = chud_core::validate::validate(&task);
    fs::set_permissions(lab.path("dst"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!report.runnable);
    assert!(report.issues.iter().any(|i| i.code == "target-denied"), "{:?}", report.issues);
}

#[cfg(windows)]
#[test]
fn a_denied_file_is_reported_and_the_rest_is_copied() {
    let lab = Lab::new();
    lab.write("src/interdit.txt", "secret");
    lab.write("src/libre.txt", "libre");
    // S-1-1-0 désigne « Tout le monde », quel que soit le compte ou la langue du système.
    let everyone = "*S-1-1-0";
    let target = lab.path("src/interdit.txt");
    let deny = std::process::Command::new("icacls")
        .arg(&target)
        .args(["/deny", &format!("{everyone}:(R)")])
        .output()
        .unwrap();
    assert!(deny.status.success());
    assert!(fs::read(&target).is_err(), "le refus d'accès n'a pas pris effet : ce test ne prouverait rien");

    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.retries = 1;
    let task = lab.store.save_task(task).unwrap();
    let record = lab.run(&task, None);
    std::process::Command::new("icacls")
        .arg(&target)
        .args(["/remove:d", everyone])
        .output()
        .unwrap();

    assert_eq!(record.status, RunStatus::CompletedWithErrors);
    assert!(lab.path("dst/libre.txt").exists());
    assert!(
        record.errors.iter().any(|e| e.error.as_ref().is_some_and(|f| f.category == "denied")),
        "{:?}",
        record.errors
    );
}

#[cfg(windows)]
#[test]
fn the_hidden_attribute_is_kept_and_the_file_can_still_be_updated() {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    let lab = Lab::new();
    lab.write("src/cache.dat", "premier");
    std::process::Command::new("attrib").arg("+h").arg(lab.path("src/cache.dat")).status().unwrap();
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_ne!(fs::metadata(lab.path("dst/cache.dat")).unwrap().file_attributes() & HIDDEN, 0);

    // Remplacer un fichier caché ne doit pas échouer à la mise à jour suivante.
    std::process::Command::new("attrib").arg("-h").arg(lab.path("src/cache.dat")).status().unwrap();
    fs::write(lab.path("src/cache.dat"), "seconde version").unwrap();
    std::process::Command::new("attrib").arg("+h").arg(lab.path("src/cache.dat")).status().unwrap();
    let record = lab.run(&task, None);
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(fs::read_to_string(lab.path("dst/cache.dat")).unwrap(), "seconde version");
}

#[test]
fn deep_tree_with_many_files_keeps_exact_counts() {
    let lab = Lab::new();
    for i in 0..3000 {
        lab.write(&format!("src/d{}/s{}/f{i}.txt", i % 37, i % 11), "x");
    }
    let task = lab.task(Mode::Update, &[("src", "dst", "")]);
    let plan = lab.analyze(&task);
    assert_eq!(plan.totals.copy, 3000);
    let page = plan::query(
        &plan,
        &PlanQuery {
            offset: 2990,
            limit: 100,
            ..PlanQuery::default()
        },
    );
    assert_eq!((page.total, page.rows.len()), (3000, 10));
    let record = lab.run(&task, None);
    assert_eq!(record.totals.transfers, 3000);
    assert_eq!(names(&lab.path("dst")).len(), 3000);
}

/// Montée en charge, lancée à la demande : `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn one_hundred_thousand_files() {
    const FILES: usize = 100_000;
    let lab = Lab::new();
    let clock = Instant::now();
    for i in 0..FILES {
        lab.write(&format!("src/d{}/s{}/f{i}.txt", i % 311, i % 97), "x");
    }
    println!("préparation : {:.1?}", clock.elapsed());

    let mut task = lab.task(Mode::Mirror, &[("src", "dst", "")]);
    task.options.transfers = 8;
    let task = lab.store.save_task(task).unwrap();

    let clock = Instant::now();
    let plan = lab.analyze(&task);
    println!("analyse : {:.1?}", clock.elapsed());
    assert_eq!(plan.totals.copy, FILES as u64);

    let clock = Instant::now();
    let page = plan::query(
        &plan,
        &PlanQuery {
            search: "f99999".into(),
            limit: 100,
            ..PlanQuery::default()
        },
    );
    println!("recherche dans le plan : {:.1?}", clock.elapsed());
    assert_eq!(page.total, 1);

    let clock = Instant::now();
    let record = lab.run(&task, Some(plan.clone()));
    println!("copie : {:.1?}", clock.elapsed());
    assert_eq!(record.status, RunStatus::Completed, "{:?}", record.errors);
    assert_eq!(record.totals.transfers, FILES as u64);

    let clock = Instant::now();
    let again = lab.analyze(&lab.store.save_task(task).unwrap());
    println!("seconde analyse, tout identique : {:.1?}", clock.elapsed());
    assert_eq!(again.totals.same, FILES as u64);
}

#[test]
fn options_defaults_are_safe() {
    let options = Options::default();
    assert_eq!(options.symlinks, Symlinks::Skip);
    assert!(!options.allow_merge);
    assert_eq!(Mode::default(), Mode::Update);
}

#[cfg(windows)]
#[test]
fn a_locked_file_is_reported_and_the_rest_is_copied() {
    use std::os::windows::fs::OpenOptionsExt;
    let lab = Lab::new();
    lab.write("src/verrou.txt", "occupé");
    lab.write("src/libre.txt", "libre");
    // Ouverture sans partage : ce que fait une application qui garde un fichier verrouillé.
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(lab.path("src/verrou.txt"))
        .unwrap();

    let mut task = lab.task(Mode::Update, &[("src", "dst", "")]);
    task.options.retries = 1;
    let task = lab.store.save_task(task).unwrap();
    let record = lab.run(&task, None);

    assert_eq!(record.status, RunStatus::CompletedWithErrors);
    assert!(lab.path("dst/libre.txt").exists());
    assert!(!lab.path("dst/verrou.txt").exists());
    assert!(
        record.errors.iter().any(|e| e.error.as_ref().is_some_and(|f| f.category == "locked")),
        "{:?}",
        record.errors
    );
}
