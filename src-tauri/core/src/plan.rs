//! Analyse d'une tâche : ce qui arriverait si elle était exécutée, sans rien écrire.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::engine::{Control, Engine, EngineStats, EntryKind, JobSpec, PlanEntry};
use crate::errors::FriendlyError;
use crate::model::{ResolvedLink, Task};
use crate::schedule::{run_jobs, Job};
use crate::validate::{self, Issue, Report, Severity};
use crate::{fail_text, filters, new_id, now_ms, paths, ui_text, volumes, Result};

/// Au-delà de cette part de la destination, une suppression demande une confirmation renforcée.
const MASSIVE_RATIO: f64 = 0.5;
const MASSIVE_MIN_FILES: u64 = 10;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub copy: u64,
    pub copy_bytes: u64,
    pub update: u64,
    pub update_bytes: u64,
    pub same: u64,
    pub ignored: u64,
    pub ignored_bytes: u64,
    pub delete: u64,
    pub delete_bytes: u64,
    pub extra: u64,
    pub errors: u64,
    pub conflicts: u64,
    pub incomplete: u64,
}

impl Counts {
    pub fn transfer_files(&self) -> u64 {
        self.copy + self.update
    }

    pub fn transfer_bytes(&self) -> u64 {
        self.copy_bytes + self.update_bytes
    }

    fn add(&mut self, other: &Counts) {
        self.copy += other.copy;
        self.copy_bytes += other.copy_bytes;
        self.update += other.update;
        self.update_bytes += other.update_bytes;
        self.same += other.same;
        self.ignored += other.ignored;
        self.ignored_bytes += other.ignored_bytes;
        self.delete += other.delete;
        self.delete_bytes += other.delete_bytes;
        self.extra += other.extra;
        self.errors += other.errors;
        self.conflicts += other.conflicts;
        self.incomplete += other.incomplete;
    }

    fn of(entries: &[PlanEntry], errors: usize) -> Counts {
        let mut counts = Counts {
            errors: errors as u64,
            ..Counts::default()
        };
        for entry in entries {
            let size = entry.size.unwrap_or(0);
            match entry.kind {
                EntryKind::Copy => {
                    counts.copy += 1;
                    counts.copy_bytes += size;
                }
                EntryKind::Update => {
                    counts.update += 1;
                    counts.update_bytes += size;
                }
                EntryKind::Same => counts.same += 1,
                EntryKind::Ignored => {
                    counts.ignored += 1;
                    counts.ignored_bytes += size;
                }
                EntryKind::Delete => {
                    counts.delete += 1;
                    counts.delete_bytes += size;
                }
                EntryKind::Extra => counts.extra += 1,
                EntryKind::Error => counts.errors += 1,
                EntryKind::Conflict => counts.conflicts += 1,
                EntryKind::Incomplete => counts.incomplete += 1,
            }
        }
        counts
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPlan {
    pub link_id: String,
    pub source: String,
    pub target: String,
    pub analyzed: bool,
    pub skip_reason: Option<String>,
    pub counts: Counts,
    pub errors: Vec<FriendlyError>,
    /// Le moteur a échoué : le plan de cette liaison ne doit pas servir à supprimer.
    pub failed: bool,
    pub massive_delete: bool,
    pub source_looks_empty: bool,
    /// Fichiers présents dans la source mais absents de ce que le moteur a vu.
    pub unseen_files: u64,
    #[serde(skip)]
    pub entries: Vec<PlanEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceCheck {
    pub mount: String,
    pub available: u64,
    pub needed: u64,
    pub enough: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub task_updated_at: i64,
    pub created_at: i64,
    pub totals: Counts,
    pub links: Vec<LinkPlan>,
    pub space: Vec<SpaceCheck>,
    pub issues: Vec<Issue>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanProgress {
    pub link_id: String,
    pub listed: u64,
    pub checks: u64,
    pub total_checks: u64,
    pub checking: Vec<String>,
    pub finished: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanQuery {
    pub link_id: Option<String>,
    pub kinds: Vec<EntryKind>,
    pub search: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSide {
    pub size: u64,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRow {
    pub link_id: String,
    pub rel: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub note: Option<String>,
    pub source: Option<FileSide>,
    pub target: Option<FileSide>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPage {
    pub total: usize,
    pub rows: Vec<PlanRow>,
}

pub fn job_for(task: &Task, link: &ResolvedLink, max_delete: Option<u64>, stamp: &str) -> JobSpec {
    JobSpec {
        id: link.id.clone(),
        source: link.source.clone(),
        target: link.target.clone(),
        mode: task.mode,
        options: task.options.clone(),
        filters: task.filters.clone(),
        max_delete,
        archive_stamp: stamp.to_string(),
    }
}

/// Les disques sollicités par une liaison.
pub fn link_volumes(link: &ResolvedLink, disks: &[volumes::Disk]) -> Vec<String> {
    let mut out = vec![volumes::volume_key(&link.source, disks), volumes::volume_key(&link.target, disks)];
    out.dedup();
    out
}

pub fn analyze_task(engine: &dyn Engine, task: &Task, control: &Control, on_progress: &(dyn Fn(PlanProgress) + Sync)) -> Result<Plan> {
    let report = validate::validate(task);
    if report.issues.iter().any(|i| i.severity == Severity::Error) {
        return fail_text("err.analyze-blocked");
    }

    let disks = volumes::all();
    let resolved: Vec<ResolvedLink> = task.resolved_links().into_iter().filter(|l| l.enabled).collect();
    let results: Mutex<Vec<LinkPlan>> = Mutex::new(Vec::new());
    let mut jobs = Vec::new();

    for link in &resolved {
        match not_ready_reason(&report, &link.id) {
            Some(reason) => results.lock().unwrap().push(skipped(link, reason)),
            None => jobs.push(Job {
                volumes: link_volumes(link, &disks),
                item: link.clone(),
            }),
        }
    }

    run_jobs(
        jobs,
        task.options.parallel_volumes,
        &|| control.is_cancelled(),
        &|link: ResolvedLink| {
            let job = job_for(task, &link, None, "analyse");
            let link_id = link.id.clone();
            let mut report_stats = |stats: &EngineStats| {
                on_progress(PlanProgress {
                    link_id: link_id.clone(),
                    listed: stats.listed,
                    checks: stats.checks,
                    total_checks: stats.total_checks,
                    checking: stats.checking.clone(),
                    finished: false,
                });
            };
            // Le compte de contrôle des fichiers de la source se fait pendant l'analyse, pas après :
            // sur un partage réseau, une seconde lecture complète de l'arborescence doublait la durée.
            let abandon = std::sync::atomic::AtomicBool::new(false);
            let (analysis, on_disk) = std::thread::scope(|scope| {
                let stop = || control.is_cancelled() || abandon.load(std::sync::atomic::Ordering::Relaxed);
                let (source, wanted) = (&link.source, &task.filters);
                let counter = scope.spawn(move || filters::count_source_files(source, wanted, &stop));
                let analysis = engine.analyze(&job, control, &mut report_stats);
                if !matches!(&analysis, Ok(a) if !a.failed && !a.cancelled) {
                    abandon.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                (analysis, counter.join().ok().flatten())
            });
            let plan = match analysis {
                Ok(analysis) => {
                    let trustworthy = !analysis.failed && !analysis.cancelled;
                    let failed = analysis.failed;
                    let mut link_plan = finish_link(&link, analysis.entries, analysis.errors);
                    link_plan.failed = failed;
                    if let (true, Some(on_disk)) = (trustworthy, on_disk) {
                        link_plan.unseen_files = unseen_by_engine(&link_plan, on_disk);
                    }
                    link_plan
                }
                Err(e) => {
                    let mut link_plan = skipped(&link, e.to_string());
                    link_plan.failed = true;
                    link_plan
                }
            };
            on_progress(PlanProgress {
                link_id: link.id.clone(),
                finished: true,
                ..PlanProgress::default()
            });
            results.lock().unwrap().push(plan);
        },
        &|link: ResolvedLink| results.lock().unwrap().push(skipped(&link, ui_text("msg.analysis-cancelled", &[]))),
    );

    let mut links = results.into_inner().unwrap_or_default();
    let order: HashMap<&str, usize> = resolved.iter().enumerate().map(|(i, l)| (l.id.as_str(), i)).collect();
    links.sort_by_key(|l| order.get(l.link_id.as_str()).copied().unwrap_or(usize::MAX));

    detect_collisions(&mut links, &disks);
    for link in &mut links {
        link.counts = Counts::of(&link.entries, link.errors.len());
    }

    let mut totals = Counts::default();
    for link in &links {
        totals.add(&link.counts);
    }

    let space = space_checks(&links, &disks);
    let mut issues = report.issues;
    for link in &links {
        if link.unseen_files > 0 {
            let mirror = task.mode.removes_from_destination();
            let (severity, code) = if mirror {
                (Severity::Error, "unseen-files-mirror")
            } else {
                (Severity::Warning, "unseen-files")
            };
            let params = [("n", link.unseen_files.to_string()), ("path", link.source.clone())];
            issues.push(Issue::new(severity, code, &params, Some(&link.link_id)));
        }
        if link.source_looks_empty {
            let params = [("path", link.source.clone()), ("n", link.counts.delete.to_string())];
            issues.push(Issue::new(Severity::Warning, "empty-source", &params, Some(&link.link_id)));
        } else if link.massive_delete {
            let params = [("n", link.counts.delete.to_string()), ("path", link.target.clone())];
            issues.push(Issue::new(Severity::Warning, "massive-delete", &params, Some(&link.link_id)));
        }
    }
    for check in &space {
        if !check.enough {
            issues.push(Issue::new(Severity::Warning, "low-space", &[("path", check.mount.clone())], None));
        }
    }

    Ok(Plan {
        id: new_id(),
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        task_updated_at: task.updated_at,
        created_at: now_ms(),
        totals,
        links,
        space,
        issues,
        cancelled: control.is_cancelled(),
    })
}

fn not_ready_reason(report: &Report, link_id: &str) -> Option<String> {
    let link = report.links.iter().find(|l| l.id == link_id)?;
    if link.ready {
        return None;
    }
    let issue = report.issues.iter().find(|i| i.link_id.as_deref() == Some(link_id));
    Some(issue.map(Issue::text).unwrap_or_else(|| ui_text("msg.link-unavailable", &[])))
}

fn skipped(link: &ResolvedLink, reason: String) -> LinkPlan {
    LinkPlan {
        link_id: link.id.clone(),
        source: link.source.display().to_string(),
        target: link.target.display().to_string(),
        analyzed: false,
        skip_reason: Some(reason),
        counts: Counts::default(),
        errors: Vec::new(),
        failed: false,
        massive_delete: false,
        source_looks_empty: false,
        unseen_files: 0,
        entries: Vec::new(),
    }
}

/// Un moteur qui passe des fichiers sous silence rendrait une sauvegarde incomplète sans le dire,
/// et un miroir supprimerait leur copie : son compte est donc recoupé avec celui du disque.
fn unseen_by_engine(plan: &LinkPlan, on_disk: u64) -> u64 {
    let unreadable = plan.entries.iter().filter(|e| e.kind == EntryKind::Error).count() as u64;
    let counts = &plan.counts;
    on_disk.saturating_sub(counts.copy + counts.update + counts.same + counts.ignored + unreadable)
}

fn finish_link(link: &ResolvedLink, entries: Vec<PlanEntry>, errors: Vec<FriendlyError>) -> LinkPlan {
    let counts = Counts::of(&entries, errors.len());
    let destination_files = counts.delete + counts.same + counts.update;
    let source_files = counts.copy + counts.update + counts.same;
    let ratio = if destination_files > 0 {
        counts.delete as f64 / destination_files as f64
    } else {
        0.0
    };
    LinkPlan {
        link_id: link.id.clone(),
        source: link.source.display().to_string(),
        target: link.target.display().to_string(),
        analyzed: true,
        skip_reason: None,
        massive_delete: counts.delete >= MASSIVE_MIN_FILES && ratio >= MASSIVE_RATIO,
        source_looks_empty: source_files == 0 && counts.delete > 0,
        counts,
        errors,
        failed: false,
        unseen_files: 0,
        entries,
    }
}

/// Signale les fichiers qui s'écraseraient entre eux.
fn detect_collisions(links: &mut [LinkPlan], disks: &[volumes::Disk]) {
    // Fusion autorisée : le même chemin relatif arrive de deux sources vers le même dossier.
    let mut by_target: HashMap<Vec<String>, HashMap<String, String>> = HashMap::new();
    let mut extra: Vec<(usize, PlanEntry)> = Vec::new();
    for (index, link) in links.iter().enumerate() {
        let target_key = paths::key_of_text(&link.target);
        let seen = by_target.entry(target_key).or_default();
        for entry in &link.entries {
            if !matches!(entry.kind, EntryKind::Copy | EntryKind::Update | EntryKind::Same) {
                continue;
            }
            match seen.get(&entry.rel) {
                Some(first) => extra.push((
                    index,
                    PlanEntry {
                        rel: entry.rel.clone(),
                        kind: EntryKind::Conflict,
                        size: entry.size,
                        note: Some(ui_text("note.merge-collision", &[("path", first.clone())])),
                    },
                )),
                None => {
                    seen.insert(entry.rel.clone(), link.source.clone());
                }
            }
        }
    }

    // Un système de fichiers qui ignore la casse ne peut pas garder `A.txt` et `a.txt`.
    for (index, link) in links.iter().enumerate() {
        if !ignores_case(std::path::Path::new(&link.target), disks) {
            continue;
        }
        let mut lowered: HashMap<String, &str> = HashMap::new();
        for entry in &link.entries {
            if !matches!(entry.kind, EntryKind::Copy | EntryKind::Update | EntryKind::Same) {
                continue;
            }
            if let Some(first) = lowered.insert(entry.rel.to_lowercase(), &entry.rel) {
                if first != entry.rel {
                    extra.push((
                        index,
                        PlanEntry {
                            rel: entry.rel.clone(),
                            kind: EntryKind::Conflict,
                            size: entry.size,
                            note: Some(ui_text("note.case-collision", &[("path", first.to_string())])),
                        },
                    ));
                }
            }
        }
    }

    for (index, entry) in extra {
        links[index].entries.push(entry);
    }
}

fn ignores_case(target: &std::path::Path, disks: &[volumes::Disk]) -> bool {
    if cfg!(windows) {
        return true;
    }
    volumes::containing(target, disks)
        .map(|d| matches!(d.file_system.to_lowercase().as_str(), "ntfs" | "ntfs3" | "fuseblk" | "vfat" | "exfat" | "msdos"))
        .unwrap_or(false)
}

fn space_checks(links: &[LinkPlan], disks: &[volumes::Disk]) -> Vec<SpaceCheck> {
    let mut needed: HashMap<String, (u64, u64)> = HashMap::new();
    for link in links.iter().filter(|l| l.analyzed) {
        let target = std::path::Path::new(&link.target);
        // Un partage réseau n'est pas dans la liste des disques : son espace libre est demandé
        // directement, faute de quoi un NAS trop petit ne serait découvert qu'en pleine copie.
        let (mount, available) = match volumes::containing(target, disks) {
            Some(disk) => (disk.mount, disk.available),
            None => match crate::drives::free_space_in_time(target) {
                Some(found) => found,
                None => continue,
            },
        };
        let slot = needed.entry(mount).or_insert((available, 0));
        slot.1 += link.counts.transfer_bytes();
    }
    let mut out: Vec<SpaceCheck> = needed
        .into_iter()
        .map(|(mount, (available, needed))| SpaceCheck {
            mount,
            available,
            needed,
            enough: needed <= available,
        })
        .collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

/// Les changements les plus graves d'abord : c'est ce qu'il faut voir en priorité.
fn severity_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Delete => 0,
        EntryKind::Error => 1,
        EntryKind::Conflict => 2,
        EntryKind::Incomplete => 3,
        EntryKind::Update => 4,
        EntryKind::Copy => 5,
        EntryKind::Extra => 6,
        EntryKind::Ignored => 7,
        EntryKind::Same => 8,
    }
}

fn side(path: &std::path::Path) -> Option<FileSide> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64);
    Some(FileSide { size: meta.len(), modified })
}

pub fn query(plan: &Plan, request: &PlanQuery) -> PlanPage {
    let needle = request.search.to_lowercase();
    let mut matches: Vec<(&LinkPlan, &PlanEntry)> = plan
        .links
        .iter()
        .filter(|l| request.link_id.as_ref().is_none_or(|id| *id == l.link_id))
        .flat_map(|l| l.entries.iter().map(move |e| (l, e)))
        .filter(|(_, e)| request.kinds.is_empty() || request.kinds.contains(&e.kind))
        .filter(|(_, e)| needle.is_empty() || e.rel.to_lowercase().contains(&needle))
        .collect();
    matches.sort_by(|a, b| severity_rank(a.1.kind).cmp(&severity_rank(b.1.kind)).then_with(|| a.1.rel.cmp(&b.1.rel)));

    let total = matches.len();
    let limit = if request.limit == 0 { 100 } else { request.limit.min(500) };
    let page: Vec<(&LinkPlan, &PlanEntry)> = matches.into_iter().skip(request.offset).take(limit).collect();
    // Les dates des deux côtés sont lues en parallèle : sur un partage réseau, deux cents
    // interrogations l'une après l'autre faisaient attendre chaque page plusieurs secondes.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let sides: std::sync::Mutex<Vec<(Option<FileSide>, Option<FileSide>)>> = std::sync::Mutex::new(vec![(None, None); page.len()]);
    std::thread::scope(|scope| {
        for _ in 0..page.len().clamp(1, 16) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((link, entry)) = page.get(index) else { break };
                let relative: std::path::PathBuf = entry.rel.split('/').collect();
                let found = (
                    side(&std::path::Path::new(&link.source).join(&relative)),
                    side(&std::path::Path::new(&link.target).join(&relative)),
                );
                sides.lock().unwrap_or_else(|e| e.into_inner())[index] = found;
            });
        }
    });
    let sides = sides.into_inner().unwrap_or_else(|e| e.into_inner());
    let rows = page
        .into_iter()
        .zip(sides)
        .map(|((link, entry), (source, target))| PlanRow {
            link_id: link.link_id.clone(),
            rel: entry.rel.clone(),
            kind: entry.kind,
            size: entry.size,
            note: entry.note.clone(),
            source,
            target,
        })
        .collect();
    PlanPage { total, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: &str, kind: EntryKind, size: u64) -> PlanEntry {
        PlanEntry {
            rel: rel.into(),
            kind,
            size: Some(size),
            note: None,
        }
    }

    fn link(id: &str, entries: Vec<PlanEntry>) -> LinkPlan {
        let resolved = ResolvedLink {
            id: id.into(),
            source: format!("/s/{id}").into(),
            destination_root: "/d".into(),
            target: "/d".into(),
            enabled: true,
        };
        finish_link(&resolved, entries, Vec::new())
    }

    #[test]
    fn counts_split_by_kind() {
        let counts = Counts::of(
            &[
                entry("a", EntryKind::Copy, 10),
                entry("b", EntryKind::Update, 5),
                entry("c", EntryKind::Delete, 7),
                entry("d", EntryKind::Same, 1),
            ],
            2,
        );
        assert_eq!(counts.transfer_bytes(), 15);
        assert_eq!(counts.transfer_files(), 2);
        assert_eq!(counts.delete_bytes, 7);
        assert_eq!(counts.errors, 2);
    }

    #[test]
    fn deleting_a_few_files_is_not_massive() {
        assert!(!link("a", vec![entry("x", EntryKind::Delete, 1), entry("y", EntryKind::Same, 1)]).massive_delete);
    }

    #[test]
    fn deleting_most_of_a_folder_is_massive() {
        let entries = (0..12).map(|i| entry(&format!("{i}"), EntryKind::Delete, 1)).collect();
        assert!(link("a", entries).massive_delete);
    }

    #[test]
    fn an_empty_source_facing_a_full_destination_is_flagged() {
        assert!(link("a", vec![entry("x", EntryKind::Delete, 1)]).source_looks_empty);
    }

    #[test]
    fn merged_sources_report_their_collisions() {
        let mut links = vec![
            link("a", vec![entry("commun.txt", EntryKind::Copy, 1)]),
            link("b", vec![entry("commun.txt", EntryKind::Copy, 2), entry("autre.txt", EntryKind::Copy, 3)]),
        ];
        detect_collisions(&mut links, &[]);
        assert_eq!(links[1].entries.iter().filter(|e| e.kind == EntryKind::Conflict).count(), 1);
    }

    #[test]
    fn deletions_come_first_in_results() {
        let plan = Plan {
            id: "p".into(),
            task_id: "t".into(),
            task_name: "n".into(),
            task_updated_at: 0,
            created_at: 0,
            totals: Counts::default(),
            links: vec![link("a", vec![entry("a", EntryKind::Copy, 1), entry("z", EntryKind::Delete, 1)])],
            space: Vec::new(),
            issues: Vec::new(),
            cancelled: false,
        };
        let page = query(&plan, &PlanQuery::default());
        assert_eq!(page.rows[0].rel, "z");
        let filtered = query(
            &plan,
            &PlanQuery {
                search: "A".into(),
                kinds: vec![EntryKind::Copy],
                ..PlanQuery::default()
            },
        );
        assert_eq!(filtered.total, 1);
    }
}
