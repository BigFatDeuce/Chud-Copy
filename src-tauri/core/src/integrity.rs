//! Contrôle d'intégrité d'un dossier ou d'un disque, à la manière de MD5summer.
//!
//! Chaque fichier est lu en entier et son empreinte MD5 comparée à celle du contrôle précédent.
//! Rien n'est jamais écrit dans le dossier contrôlé : les empreintes de référence restent dans
//! les données de l'application. Un contenu qui change alors que sa taille et sa date sont
//! identiques ne vient pas d'une modification normale : c'est le signe d'une corruption.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::engine::Control;
use crate::filters::{is_system_dir, HOLDING_DIR};
use crate::store::{atomic_write, Store};
use crate::{fail_text, now_ms, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSum {
    pub rel: String,
    pub size: u64,
    /// Secondes depuis l'époque Unix.
    pub modified: i64,
    pub md5: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub root: String,
    pub created_at: i64,
    pub files: Vec<FileSum>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    /// Contenu différent, taille et date inchangées : corruption probable.
    Corrupt,
    /// Fichier illisible : erreur d'entrée-sortie, secteur défectueux, droits.
    Unreadable,
    /// Modifié normalement depuis la référence : sa date a changé.
    Modified,
    Missing,
    Added,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: FindingKind,
    pub rel: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Counts {
    pub corrupt: u64,
    pub unreadable: u64,
    pub modified: u64,
    pub missing: u64,
    pub added: u64,
    pub intact: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IntegrityReport {
    pub root: String,
    pub started_at: i64,
    pub elapsed: f64,
    pub files: u64,
    pub bytes: u64,
    /// Date de la référence comparée, absente lors d'un premier contrôle.
    pub baseline: Option<i64>,
    pub counts: Counts,
    /// Les constats, les plus graves d'abord, limités pour rester affichables.
    pub findings: Vec<Finding>,
    pub cancelled: bool,
    /// Vrai quand ce contrôle est devenu la nouvelle référence.
    pub saved: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityProgress {
    pub files: u64,
    pub total_files: u64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub current: String,
}

const MAX_FINDINGS: usize = 5000;

/// The identity of a checked folder. Windows and its shares ignore letter case, Linux does not:
/// there, `Music` and `music` are two different folders and must never share a reference.
fn identity(root: &Path) -> String {
    let text = root.display().to_string();
    let trimmed = text.trim_end_matches(['/', '\\']);
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn key(root: &Path) -> String {
    format!("{:x}", Md5::digest(identity(root).as_bytes()))
}

/// The key used by versions up to 1.8.3 on every system: on Linux it merged folders that only
/// differ by letter case.
#[cfg(unix)]
fn legacy_key(root: &Path) -> String {
    let text = root.display().to_string();
    format!("{:x}", Md5::digest(text.trim_end_matches(['/', '\\']).to_lowercase().as_bytes()))
}

fn named(store: &Store, key: &str, pending: bool) -> PathBuf {
    let name = if pending { format!("{key}.pending.json") } else { format!("{key}.json") };
    store.root().join("integrity").join(name)
}

fn manifest_path(store: &Store, root: &Path, pending: bool) -> PathBuf {
    named(store, &key(root), pending)
}

fn read_manifest(path: &Path) -> Option<Manifest> {
    fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// A reference is only ever compared with the folder it was made for.
fn describes(manifest: &Manifest, root: &Path) -> bool {
    identity(Path::new(&manifest.root)) == identity(root)
}

/// Moves a reference saved under the older key to the current one, only when it was made for
/// exactly this folder. A reference made for another folder sharing the old key is left alone,
/// and the old file is kept aside rather than deleted.
#[cfg(unix)]
fn migrate_legacy(store: &Store, root: &Path) {
    let old = legacy_key(root);
    if old == key(root) {
        return;
    }
    for pending in [false, true] {
        let legacy = named(store, &old, pending);
        let current = manifest_path(store, root, pending);
        if current.exists() || !read_manifest(&legacy).is_some_and(|m| describes(&m, root)) {
            continue;
        }
        if fs::copy(&legacy, &current).is_ok() {
            let _ = fs::rename(&legacy, legacy.with_extension("json.migrated"));
        }
    }
}

#[cfg(not(unix))]
fn migrate_legacy(_store: &Store, _root: &Path) {}

pub fn baseline(store: &Store, root: &Path) -> Option<Manifest> {
    migrate_legacy(store, root);
    read_manifest(&manifest_path(store, root, false)).filter(|m| describes(m, root))
}

/// True when `rel` is one of the unread paths or lies inside one of them. Compared folder by
/// folder: `sous-autre/x` is not inside `sous`.
fn unobserved(rel: &str, unread: &HashSet<&str>) -> bool {
    if unread.contains("") || unread.contains(rel) {
        return true;
    }
    rel.match_indices('/').any(|(at, _)| unread.contains(&rel[..at]))
}

fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn modified_secs(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn walk(root: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_dir() && (is_system_dir(&name) || name == HOLDING_DIR))
        })
}

enum Hashed {
    Done { md5: String, size: u64, modified: i64 },
    Cancelled,
    /// The file kept changing while it was read: its digest describes no real state.
    Unstable,
}

/// Reads a file and returns its digest with the size and date it had while being read.
/// The size and date listed earlier are not reused: a file edited between the listing and the
/// reading would otherwise pair a new content with an old date, which looks like a corruption.
fn hash_file(path: &Path, control: &Control, read: &mut dyn FnMut(u64)) -> std::io::Result<Hashed> {
    let mut buffer = vec![0u8; 1 << 20];
    for _attempt in 0..2 {
        let mut file = fs::File::open(path)?;
        let before = file.metadata()?;
        let mut hasher = Md5::new();
        loop {
            while control.is_paused() && !control.is_cancelled() {
                std::thread::sleep(Duration::from_millis(100));
            }
            if control.is_cancelled() {
                return Ok(Hashed::Cancelled);
            }
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            read(n as u64);
        }
        let after = file.metadata()?;
        let (size, modified) = (after.len(), modified_secs(&after));
        if (before.len(), modified_secs(&before)) == (size, modified) {
            return Ok(Hashed::Done {
                md5: format!("{:x}", hasher.finalize()),
                size,
                modified,
            });
        }
    }
    Ok(Hashed::Unstable)
}

/// Lit tout le dossier, compare à la référence et renvoie le constat.
/// Sans référence, ce contrôle la devient ; sinon il attend d'être accepté par l'utilisateur.
pub fn check(store: &Store, root: &Path, control: &Control, progress: &mut dyn FnMut(&IntegrityProgress)) -> Result<IntegrityReport> {
    if !root.is_dir() {
        return fail_text("err.path-inaccessible");
    }
    let started = Instant::now();
    let mut report = IntegrityReport {
        root: root.display().to_string(),
        started_at: now_ms(),
        ..IntegrityReport::default()
    };
    let previous = baseline(store, root);
    report.baseline = previous.as_ref().map(|m| m.created_at);

    let mut state = IntegrityProgress::default();
    let mut findings = Vec::new();
    // La taille et la date lues pendant le parcours servent ensuite : sur un partage réseau,
    // chaque nouvelle interrogation d'un fichier coûte un aller-retour avec le serveur.
    let mut files: Vec<PathBuf> = Vec::new();
    let mut last_report = Instant::now();
    for entry in walk(root) {
        match entry {
            Ok(entry) if entry.file_type().is_file() => {
                state.total_files += 1;
                state.total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                files.push(entry.into_path());
                if last_report.elapsed() >= Duration::from_millis(250) {
                    progress(&state);
                    last_report = Instant::now();
                }
            }
            Ok(_) => {}
            Err(error) => findings.push(Finding {
                kind: FindingKind::Unreadable,
                rel: error.path().map(|p| rel_of(root, p)).unwrap_or_default(),
                detail: Some(error.to_string()),
            }),
        }
        if control.is_cancelled() {
            break;
        }
    }
    progress(&state);

    let mut sums = Vec::with_capacity(files.len());
    for path in files {
        if control.is_cancelled() {
            break;
        }
        let path = &path;
        let rel = rel_of(root, path);
        state.current = rel.clone();
        let mut on_read = |n: u64| {
            state.bytes += n;
            if last_report.elapsed() >= Duration::from_millis(250) {
                progress(&state);
                last_report = Instant::now();
            }
        };
        match hash_file(path, control, &mut on_read) {
            Ok(Hashed::Done { md5, size, modified }) => sums.push(FileSum { rel, size, modified, md5 }),
            Ok(Hashed::Cancelled) => break,
            Ok(Hashed::Unstable) => findings.push(Finding {
                kind: FindingKind::Unreadable,
                rel,
                detail: Some("changed while it was being read".to_string()),
            }),
            Err(error) => findings.push(Finding {
                kind: FindingKind::Unreadable,
                rel,
                detail: Some(error.to_string()),
            }),
        }
        state.files += 1;
    }
    report.cancelled = control.is_cancelled();
    report.files = state.files;
    report.bytes = state.bytes;
    state.current.clear();
    progress(&state);

    // Taken before the findings are merged and truncated for display.
    let unread_rels: HashSet<String> = findings
        .iter()
        .filter(|f| f.kind == FindingKind::Unreadable)
        .map(|f| f.rel.clone())
        .collect();
    if let Some(previous) = &previous {
        let complete = !report.cancelled;
        compare(previous, &sums, &findings, &mut report, complete);
    } else {
        report.counts.intact = sums.len() as u64;
    }
    for finding in &findings {
        if finding.kind == FindingKind::Unreadable {
            report.counts.unreadable += 1;
        }
    }
    findings.extend(std::mem::take(&mut report.findings));
    findings.sort_by_key(|f| (f.kind as u8, f.rel.clone()));
    findings.truncate(MAX_FINDINGS);
    report.findings = findings;

    if !report.cancelled {
        let unread: HashSet<&str> = unread_rels.iter().map(String::as_str).collect();
        let partial = !unread.is_empty();
        // What could not be read this time keeps its last known digest: accepting a partial check
        // must not make the reference forget those files.
        let mut files = sums;
        if let Some(previous) = &previous {
            let seen: HashSet<String> = files.iter().map(|f| f.rel.clone()).collect();
            files.extend(
                previous
                    .files
                    .iter()
                    .filter(|old| !seen.contains(&old.rel) && unobserved(&old.rel, &unread))
                    .cloned(),
            );
        }
        let manifest = Manifest {
            root: report.root.clone(),
            created_at: report.started_at,
            files,
        };
        // A first check that could not read everything only becomes the reference on request.
        let pending = previous.is_some() || partial;
        atomic_write(&manifest_path(store, root, pending), &serde_json::to_vec(&manifest)?)?;
        report.saved = !pending;
    }
    report.elapsed = started.elapsed().as_secs_f64();
    Ok(report)
}

fn compare(previous: &Manifest, sums: &[FileSum], findings: &[Finding], report: &mut IntegrityReport, complete: bool) {
    let before: HashMap<&str, &FileSum> = previous.files.iter().map(|f| (f.rel.as_str(), f)).collect();
    let now: HashMap<&str, &FileSum> = sums.iter().map(|f| (f.rel.as_str(), f)).collect();
    let unread: HashSet<&str> = findings
        .iter()
        .filter(|f| f.kind == FindingKind::Unreadable)
        .map(|f| f.rel.as_str())
        .collect();
    for current in sums {
        match before.get(current.rel.as_str()) {
            None => {
                report.counts.added += 1;
                report.findings.push(Finding {
                    kind: FindingKind::Added,
                    rel: current.rel.clone(),
                    detail: None,
                });
            }
            Some(old) if old.md5 == current.md5 => report.counts.intact += 1,
            Some(old) if old.size == current.size && old.modified == current.modified => {
                report.counts.corrupt += 1;
                report.findings.push(Finding {
                    kind: FindingKind::Corrupt,
                    rel: current.rel.clone(),
                    detail: Some(format!("{} → {}", old.md5, current.md5)),
                });
            }
            Some(_) => {
                report.counts.modified += 1;
                report.findings.push(Finding {
                    kind: FindingKind::Modified,
                    rel: current.rel.clone(),
                    detail: None,
                });
            }
        }
    }
    // Un contrôle interrompu n'a pas tout lu : l'absence d'un fichier n'y prouve rien.
    // Il en va de même pour tout ce qui se trouve dans un dossier illisible.
    if complete {
        for old in &previous.files {
            if !now.contains_key(old.rel.as_str()) && !unobserved(&old.rel, &unread) {
                report.counts.missing += 1;
                report.findings.push(Finding {
                    kind: FindingKind::Missing,
                    rel: old.rel.clone(),
                    detail: None,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub root: String,
    pub created_at: i64,
    pub files: u64,
    pub bytes: u64,
}

/// Les dossiers et disques déjà contrôlés, avec leur référence.
pub fn references(store: &Store) -> Vec<Reference> {
    let mut out: Vec<Reference> = fs::read_dir(store.root().join("integrity"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.to_string_lossy().ends_with(".pending.json"))
                .filter_map(|p| serde_json::from_slice::<Manifest>(&fs::read(p).ok()?).ok())
                .map(|m| Reference {
                    root: m.root.clone(),
                    created_at: m.created_at,
                    files: m.files.len() as u64,
                    bytes: m.files.iter().map(|f| f.size).sum(),
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    out
}

/// Oublie la référence d'un dossier. Seules les données de l'application sont effacées.
pub fn forget(store: &Store, root: &Path) -> Result<()> {
    migrate_legacy(store, root);
    for pending in [false, true] {
        let path = manifest_path(store, root, pending);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Fait du dernier contrôle la nouvelle référence, une fois les écarts compris par l'utilisateur.
pub fn accept(store: &Store, root: &Path) -> Result<()> {
    migrate_legacy(store, root);
    let pending = manifest_path(store, root, true);
    if !pending.exists() {
        return fail_text("err.integrity-nothing");
    }
    fs::rename(pending, manifest_path(store, root, false))?;
    Ok(())
}

/// Écrit la référence au format de MD5summer et de md5sum, là où l'utilisateur l'a choisi.
pub fn export_md5(store: &Store, root: &Path, target: &Path) -> Result<usize> {
    let Some(manifest) = baseline(store, root) else {
        return fail_text("err.integrity-nothing");
    };
    let separator = if cfg!(windows) { "\\" } else { "/" };
    let mut text = String::new();
    for file in &manifest.files {
        text.push_str(&format!("{} *{}\r\n", file.md5, file.rel.replace('/', separator)));
    }
    atomic_write(target, text.as_bytes())?;
    Ok(manifest.files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab() -> (tempfile::TempDir, Store, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("donnees")).unwrap();
        let root = dir.path().join("disque");
        fs::create_dir_all(root.join("sous")).unwrap();
        fs::write(root.join("a.txt"), "contenu a").unwrap();
        fs::write(root.join("sous/b.bin"), vec![7u8; 300_000]).unwrap();
        fs::write(root.join("sous/c.txt"), "contenu c").unwrap();
        (dir, store, root)
    }

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<_> = walkdir::WalkDir::new(root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| (rel_of(root, e.path()), fs::read(e.path()).unwrap()))
            .collect();
        out.sort();
        out
    }

    /// Change le contenu d'un fichier sans toucher à sa taille ni à sa date, comme une corruption.
    fn corrupt(path: &Path) {
        let metadata = fs::metadata(path).unwrap();
        let mut bytes = fs::read(path).unwrap();
        bytes[0] ^= 0xff;
        fs::write(path, bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(metadata.modified().unwrap())
            .unwrap();
    }

    #[test]
    fn a_first_check_becomes_the_reference_without_touching_the_folder() {
        let (_dir, store, root) = lab();
        let before = snapshot(&root);
        let report = check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.files, 3);
        assert!(report.saved && report.baseline.is_none());
        assert_eq!(report.counts.intact, 3);
        assert_eq!(snapshot(&root), before, "le dossier contrôlé n'est jamais modifié");
        assert_eq!(baseline(&store, &root).unwrap().files.len(), 3);
    }

    #[test]
    fn silent_corruption_is_told_apart_from_normal_changes() {
        let (_dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();

        corrupt(&root.join("sous/b.bin"));
        std::thread::sleep(Duration::from_millis(1100));
        fs::write(root.join("a.txt"), "contenu a, modifié").unwrap();
        fs::remove_file(root.join("sous/c.txt")).unwrap();
        fs::write(root.join("nouveau.txt"), "n").unwrap();

        let report = check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let kind = |rel: &str| report.findings.iter().find(|f| f.rel == rel).map(|f| f.kind);
        assert_eq!(kind("sous/b.bin"), Some(FindingKind::Corrupt));
        assert_eq!(kind("a.txt"), Some(FindingKind::Modified));
        assert_eq!(kind("sous/c.txt"), Some(FindingKind::Missing));
        assert_eq!(kind("nouveau.txt"), Some(FindingKind::Added));
        assert_eq!(report.findings[0].kind, FindingKind::Corrupt, "le plus grave d'abord");
        assert!(!report.saved, "une référence n'est remplacée que sur accord");

        accept(&store, &root).unwrap();
        let again = check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(again.counts.corrupt + again.counts.modified + again.counts.missing + again.counts.added, 0);
    }

    #[test]
    fn a_cancelled_check_reports_no_missing_file_and_keeps_the_reference() {
        let (_dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let control = Control::default();
        control.cancel();
        let report = check(&store, &root, &control, &mut |_| {}).unwrap();
        assert!(report.cancelled);
        assert_eq!(report.counts.missing, 0);
        assert!(!manifest_path(&store, &root, true).exists());
    }

    #[test]
    fn references_are_listed_and_can_be_forgotten() {
        let (_dir, store, root) = lab();
        assert!(references(&store).is_empty());
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let listed = references(&store);
        assert_eq!((listed.len(), listed[0].files), (1, 3));
        let before = snapshot(&root);
        forget(&store, &root).unwrap();
        assert!(references(&store).is_empty());
        assert!(baseline(&store, &root).is_none());
        assert_eq!(snapshot(&root), before, "oublier une référence ne touche pas au dossier");
    }

    /// Makes a folder unreadable for the duration of a test. Returns false when the platform or
    /// the account (root under Linux) ignores the restriction: the test would then prove nothing.
    fn deny(dir: &Path) -> bool {
        #[cfg(windows)]
        {
            // S-1-1-0 is "Everyone", whatever the account or the system language.
            let denied = std::process::Command::new("icacls")
                .arg(dir)
                .args(["/deny", "*S-1-1-0:(RD)"])
                .output()
                .is_ok_and(|o| o.status.success());
            if !denied {
                return false;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        }
        fs::read_dir(dir).is_err()
    }

    fn allow(dir: &Path) {
        #[cfg(windows)]
        let _ = std::process::Command::new("icacls").arg(dir).args(["/remove:d", "*S-1-1-0"]).output();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn files_inside_an_unreadable_folder_are_not_reported_missing() {
        let (_dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        if !deny(&root.join("sous")) {
            allow(&root.join("sous"));
            eprintln!("skipped: this account ignores folder permissions");
            return;
        }
        let report = check(&store, &root, &Control::default(), &mut |_| {});
        allow(&root.join("sous"));
        let report = report.unwrap();
        assert_eq!(report.counts.missing, 0, "{:?}", report.findings);
        assert!(report.counts.unreadable >= 1);
        assert!(!report.saved);
        // Accepting this partial check must not forget the files that could not be read.
        accept(&store, &root).unwrap();
        let kept: Vec<_> = baseline(&store, &root).unwrap().files.into_iter().map(|f| f.rel).collect();
        assert!(kept.contains(&"sous/b.bin".to_string()) && kept.contains(&"sous/c.txt".to_string()), "{kept:?}");
    }

    #[test]
    fn unread_descendants_are_never_missing_but_lookalike_names_are() {
        let file = |rel: &str| FileSum {
            rel: rel.into(),
            size: 1,
            modified: 1,
            md5: "x".into(),
        };
        let previous = Manifest {
            root: "r".into(),
            created_at: 1,
            files: vec![file("sous/b.bin"), file("sous/deep/c.txt"), file("sous-autre/d.txt"), file("a.txt")],
        };
        let findings = vec![Finding {
            kind: FindingKind::Unreadable,
            rel: "sous".into(),
            detail: None,
        }];
        let mut report = IntegrityReport::default();
        compare(&previous, &[file("a.txt")], &findings, &mut report, true);
        let missing: Vec<_> = report.findings.iter().filter(|f| f.kind == FindingKind::Missing).map(|f| f.rel.as_str()).collect();
        assert_eq!(missing, ["sous-autre/d.txt"], "a prefix of characters is not a parent folder");
    }

    #[test]
    fn a_first_check_with_unreadable_parts_waits_for_consent() {
        let (_dir, store, root) = lab();
        if !deny(&root.join("sous")) {
            allow(&root.join("sous"));
            eprintln!("skipped: this account ignores folder permissions");
            return;
        }
        let report = check(&store, &root, &Control::default(), &mut |_| {});
        allow(&root.join("sous"));
        let report = report.unwrap();
        assert!(report.counts.unreadable >= 1);
        assert!(!report.saved, "an incomplete first check must not silently become the reference");
        assert!(baseline(&store, &root).is_none());
        accept(&store, &root).unwrap();
        assert!(baseline(&store, &root).is_some(), "the user can still accept it knowingly");
    }

    #[test]
    fn a_file_changed_after_listing_is_modified_not_corrupt() {
        let (_dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let target = root.join("a.txt");
        let mut changed = false;
        // The folder is listed first, then read: the change lands between the two.
        let report = check(&store, &root, &Control::default(), &mut |state| {
            if !changed && state.total_files == 3 && state.files == 0 {
                changed = true;
                fs::write(&target, "contenu A").unwrap();
                let later = std::time::SystemTime::now() + Duration::from_secs(5);
                fs::File::options().write(true).open(&target).unwrap().set_modified(later).unwrap();
            }
        })
        .unwrap();
        assert!(changed);
        let kind = report.findings.iter().find(|f| f.rel == "a.txt").map(|f| f.kind);
        assert_eq!(kind, Some(FindingKind::Modified), "a normal edit must never be called a corruption");
        assert_eq!(report.counts.corrupt, 0);
    }

    #[test]
    fn a_reference_is_only_used_for_the_folder_it_describes() {
        let (dir, store, root) = lab();
        let other = dir.path().join("autre");
        let stranger = Manifest {
            root: other.display().to_string(),
            created_at: 1,
            files: vec![],
        };
        atomic_write(&manifest_path(&store, &root, false), &serde_json::to_vec(&stranger).unwrap()).unwrap();
        assert!(baseline(&store, &root).is_none(), "another folder's reference must never be compared");
    }

    #[cfg(windows)]
    #[test]
    fn letter_case_does_not_split_a_reference_on_windows() {
        let (_dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let shouted = PathBuf::from(root.display().to_string().to_uppercase());
        assert!(baseline(&store, &shouted).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn folders_differing_only_by_case_keep_separate_references_on_linux() {
        let (dir, store, _) = lab();
        let upper = dir.path().join("Music");
        let lower = dir.path().join("music");
        fs::create_dir_all(&upper).unwrap();
        fs::create_dir_all(&lower).unwrap();
        fs::write(upper.join("x.txt"), "upper").unwrap();
        fs::write(lower.join("y.txt"), "lower").unwrap();
        if fs::read_dir(&upper).unwrap().count() != 1 {
            return; // Case-insensitive file system: the two names are one folder.
        }
        check(&store, &upper, &Control::default(), &mut |_| {}).unwrap();
        let report = check(&store, &lower, &Control::default(), &mut |_| {}).unwrap();
        assert!(report.saved && report.baseline.is_none(), "music must not be compared with Music");
        assert_eq!(baseline(&store, &upper).unwrap().files[0].rel, "x.txt");
        assert_eq!(baseline(&store, &lower).unwrap().files[0].rel, "y.txt");
    }

    #[cfg(unix)]
    #[test]
    fn a_reference_saved_by_an_older_version_is_kept_on_linux() {
        let (_dir, store, root) = lab();
        let legacy = Manifest {
            root: root.display().to_string(),
            created_at: 7,
            files: vec![],
        };
        let path = store.root().join("integrity").join(format!("{}.json", legacy_key(&root)));
        atomic_write(&path, &serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(baseline(&store, &root).map(|m| m.created_at), Some(7));
        assert_eq!(references(&store).len(), 1, "the same folder is listed once");
    }

    #[test]
    fn the_reference_exports_in_md5summer_format() {
        let (dir, store, root) = lab();
        check(&store, &root, &Control::default(), &mut |_| {}).unwrap();
        let target = dir.path().join("export.md5");
        assert_eq!(export_md5(&store, &root, &target).unwrap(), 3);
        let text = fs::read_to_string(target).unwrap();
        let separator = if cfg!(windows) { "\\" } else { "/" };
        assert!(text.contains(&format!("{:x} *a.txt", Md5::digest(b"contenu a"))));
        assert!(text.contains(&format!(" *sous{separator}b.bin")));
    }
}
