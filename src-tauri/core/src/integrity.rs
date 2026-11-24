//! Contrôle d'intégrité d'un dossier ou d'un disque, à la manière de MD5summer.
//!
//! Chaque fichier est lu en entier et son empreinte MD5 comparée à celle du contrôle précédent.
//! Rien n'est jamais écrit dans le dossier contrôlé : les empreintes de référence restent dans
//! les données de l'application. Un contenu qui change alors que sa taille et sa date sont
//! identiques ne vient pas d'une modification normale : c'est le signe d'une corruption.

use std::collections::HashMap;
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

fn key(root: &Path) -> String {
    let normalized = root.display().to_string().trim_end_matches(['/', '\\']).to_lowercase();
    format!("{:x}", Md5::digest(normalized.as_bytes()))
}

fn manifest_path(store: &Store, root: &Path, pending: bool) -> PathBuf {
    let name = if pending {
        format!("{}.pending.json", key(root))
    } else {
        format!("{}.json", key(root))
    };
    store.root().join("integrity").join(name)
}

pub fn baseline(store: &Store, root: &Path) -> Option<Manifest> {
    fs::read(manifest_path(store, root, false))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
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

fn hash_file(path: &Path, control: &Control, read: &mut dyn FnMut(u64)) -> std::io::Result<Option<String>> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        while control.is_paused() && !control.is_cancelled() {
            std::thread::sleep(Duration::from_millis(100));
        }
        if control.is_cancelled() {
            return Ok(None);
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        read(n as u64);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
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
    let mut files: Vec<(PathBuf, Option<fs::Metadata>)> = Vec::new();
    let mut last_report = Instant::now();
    for entry in walk(root) {
        match entry {
            Ok(entry) if entry.file_type().is_file() => {
                let metadata = entry.metadata().ok();
                state.total_files += 1;
                state.total_bytes += metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                files.push((entry.into_path(), metadata));
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
    for (path, known) in files {
        if control.is_cancelled() {
            break;
        }
        let path = &path;
        let rel = rel_of(root, path);
        state.current = rel.clone();
        let metadata = match known.map(Ok).unwrap_or_else(|| fs::metadata(path)) {
            Ok(metadata) => metadata,
            Err(error) => {
                findings.push(Finding {
                    kind: FindingKind::Unreadable,
                    rel,
                    detail: Some(error.to_string()),
                });
                continue;
            }
        };
        let mut on_read = |n: u64| {
            state.bytes += n;
            if last_report.elapsed() >= Duration::from_millis(250) {
                progress(&state);
                last_report = Instant::now();
            }
        };
        match hash_file(path, control, &mut on_read) {
            Ok(Some(md5)) => sums.push(FileSum {
                rel,
                size: metadata.len(),
                modified: modified_secs(&metadata),
                md5,
            }),
            Ok(None) => break,
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
        let manifest = Manifest {
            root: report.root.clone(),
            created_at: report.started_at,
            files: sums,
        };
        let pending = previous.is_some();
        atomic_write(&manifest_path(store, root, pending), &serde_json::to_vec(&manifest)?)?;
        report.saved = !pending;
    }
    report.elapsed = started.elapsed().as_secs_f64();
    Ok(report)
}

fn compare(previous: &Manifest, sums: &[FileSum], findings: &[Finding], report: &mut IntegrityReport, complete: bool) {
    let before: HashMap<&str, &FileSum> = previous.files.iter().map(|f| (f.rel.as_str(), f)).collect();
    let now: HashMap<&str, &FileSum> = sums.iter().map(|f| (f.rel.as_str(), f)).collect();
    let unreadable: std::collections::HashSet<&str> = findings.iter().map(|f| f.rel.as_str()).collect();
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
    if complete {
        for old in &previous.files {
            if !now.contains_key(old.rel.as_str()) && !unreadable.contains(old.rel.as_str()) {
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
