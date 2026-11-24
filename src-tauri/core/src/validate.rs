//! Garde-fous évalués avant toute analyse ou exécution.
//!
//! Une erreur bloque l'opération. Un avertissement la laisse possible mais reste affiché.
//! Rien ici n'écrit sur le disque.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::drives::{self, DriveCheck};
use crate::model::{ResolvedLink, Task};
use crate::paths::{self, PathState};
use crate::ui_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub link_id: Option<String>,
}

impl Issue {
    pub fn new(severity: Severity, code: &str, params: &[(&str, String)], link_id: Option<&str>) -> Self {
        Self {
            severity,
            code: code.to_string(),
            params: params.iter().map(|(key, value)| (key.to_string(), value.clone())).collect(),
            link_id: link_id.map(str::to_string),
        }
    }

    /// Le même problème sous forme de message pour l'interface.
    pub fn text(&self) -> String {
        let params: Vec<(&str, String)> = self.params.iter().map(|(key, value)| (key.as_str(), value.clone())).collect();
        ui_text(&format!("issue.{}", self.code), &params)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReport {
    pub id: String,
    pub source: String,
    pub target: String,
    pub source_state: PathState,
    pub target_state: PathState,
    /// Vrai quand la liaison peut s'exécuter maintenant.
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub issues: Vec<Issue>,
    pub links: Vec<LinkReport>,
    /// Vrai si aucune erreur ne bloque et qu'au moins une liaison est prête.
    pub runnable: bool,
}

pub fn validate(task: &Task) -> Report {
    let mut issues = Vec::new();
    let error = Severity::Error;
    let warning = Severity::Warning;

    if task.name.trim().is_empty() {
        issues.push(Issue::new(error, "name", &[], None));
    }

    let resolved: Vec<ResolvedLink> = task.resolved_links();
    let active: Vec<&ResolvedLink> = resolved.iter().filter(|l| l.enabled).collect();
    if active.is_empty() {
        issues.push(Issue::new(error, "no-link", &[], None));
    }

    for source in &task.sources {
        if !task.links.iter().any(|l| l.source_id == source.id) {
            issues.push(Issue::new(Severity::Info, "unlinked-source", &[("path", source.path.clone())], None));
        }
    }
    for destination in &task.destinations {
        if !task.links.iter().any(|l| l.destination_id == destination.id) {
            issues.push(Issue::new(Severity::Info, "unlinked-destination", &[("path", destination.path.clone())], None));
        }
    }

    let mirror = task.mode.removes_from_destination();
    let mut reports = Vec::new();
    let mut keys: HashMap<&str, (Vec<String>, Vec<String>)> = HashMap::new();

    // Tous les chemins sont inspectés ensemble, sous un même délai.
    let wanted: Vec<(std::path::PathBuf, bool)> = resolved.iter().flat_map(|l| [(l.source.clone(), true), (l.target.clone(), false)]).collect();
    let mut inspected = paths::inspect_all(&wanted).into_iter();

    for link in &resolved {
        let id = Some(link.id.as_str());
        let (source_state, source_key) = inspected.next().expect("un état par source");
        let (target_state, target_key) = inspected.next().expect("un état par destination");
        let source_path = [("path", link.source.display().to_string())];
        let target_path = [("path", link.target.display().to_string())];

        if !link.enabled {
            reports.push(report(link, source_state, target_state, false));
            continue;
        }

        let raw = task.links.iter().find(|l| l.id == link.id).map(|l| l.subfolder.as_str()).unwrap_or("");
        let invalid_subfolder = raw.split(['/', '\\']).any(|part| part == "..") || std::path::Path::new(raw.trim()).is_absolute();
        if invalid_subfolder {
            issues.push(Issue::new(error, "subfolder", &[("name", raw.to_string())], id));
        }

        match source_state {
            PathState::Missing => issues.push(Issue::new(warning, "source-missing", &source_path, id)),
            PathState::Denied => issues.push(Issue::new(warning, "source-denied", &source_path, id)),
            PathState::NotADirectory => issues.push(Issue::new(error, "source-file", &source_path, id)),
            PathState::Unresponsive => issues.push(Issue::new(warning, "source-slow", &source_path, id)),
            _ => {}
        }
        match target_state {
            PathState::Missing => issues.push(Issue::new(warning, "target-missing", &target_path, id)),
            PathState::Denied => issues.push(Issue::new(warning, "target-denied", &target_path, id)),
            PathState::NotADirectory => issues.push(Issue::new(error, "target-file", &target_path, id)),
            PathState::Unresponsive => issues.push(Issue::new(warning, "target-slow", &target_path, id)),
            _ => {}
        }

        if paths::same(&source_key, &target_key) {
            issues.push(Issue::new(error, "same-path", &source_path, id));
        } else if paths::inside(&target_key, &source_key) {
            issues.push(Issue::new(error, "target-in-source", &target_path, id));
        } else if paths::inside(&source_key, &target_key) {
            issues.push(Issue::new(error, "source-in-target", &source_path, id));
        }

        if mirror {
            if paths::is_protected_key(&target_key) {
                issues.push(Issue::new(error, "protected-target", &target_path, id));
            } else if paths::is_root(&link.target) {
                issues.push(Issue::new(warning, "root-target", &target_path, id));
            }
        }

        let ready = !matches!(
            source_state,
            PathState::Missing | PathState::Denied | PathState::NotADirectory | PathState::Unresponsive
        ) && !matches!(
            target_state,
            PathState::Missing | PathState::Denied | PathState::NotADirectory | PathState::Unresponsive
        );
        reports.push(report(link, source_state, target_state, ready));
        keys.insert(link.id.as_str(), (source_key, target_key));
    }

    for (index, a) in active.iter().enumerate() {
        for b in active.iter().skip(index + 1) {
            let (Some((a_src, a_dst)), Some((b_src, b_dst))) = (keys.get(a.id.as_str()), keys.get(b.id.as_str())) else {
                continue;
            };
            let pair = Some(b.id.as_str());
            let same_target = paths::same(a_dst, b_dst);
            let shared = [("path", a.target.display().to_string())];

            if same_target && paths::same(a_src, b_src) {
                issues.push(Issue::new(error, "duplicate-link", &shared, pair));
            } else if same_target && mirror {
                issues.push(Issue::new(error, "merge-mirror", &shared, pair));
            } else if same_target && !task.options.allow_merge {
                issues.push(Issue::new(error, "merge", &shared, pair));
            } else if same_target {
                issues.push(Issue::new(warning, "merge-allowed", &shared, pair));
            } else if paths::overlap(a_dst, b_dst) {
                let (severity, code) = if mirror {
                    (error, "nested-targets-mirror")
                } else {
                    (warning, "nested-targets")
                };
                let both = [("a", a.target.display().to_string()), ("b", b.target.display().to_string())];
                issues.push(Issue::new(severity, code, &both, pair));
            }

            // Une destination ne doit jamais empiéter sur la source d'une autre liaison.
            for (written, read) in [(a, b), (b, a)] {
                let (_, written_key) = &keys[written.id.as_str()];
                let (read_key, _) = &keys[read.id.as_str()];
                if paths::overlap(written_key, read_key) {
                    let both = [("target", written.target.display().to_string()), ("source", read.source.display().to_string())];
                    issues.push(Issue::new(error, "target-touches-source", &both, Some(written.id.as_str())));
                }
            }
        }
    }

    issues.extend(drive_issues(task));

    let blocked = issues.iter().any(|i| i.severity == Severity::Error);
    let runnable = !blocked && reports.iter().any(|r| r.ready);
    Report {
        issues,
        links: reports,
        runnable,
    }
}

/// Garde d'identité : refuse toute opération si un autre disque occupe la place d'une source ou
/// d'une destination utilisée. Un disque simplement absent est déjà signalé par les chemins.
fn drive_issues(task: &Task) -> Vec<Issue> {
    let used = |id: &str, source: bool| {
        task.links
            .iter()
            .any(|l| l.enabled && if source { l.source_id == id } else { l.destination_id == id })
    };
    // Chaque disque est interrogé en parallèle : le délai d'un disque lent ne s'additionne pas.
    let checks: Vec<_> = task
        .sources
        .iter()
        .map(|e| (e, true))
        .chain(task.destinations.iter().map(|e| (e, false)))
        .filter(|(endpoint, source)| endpoint.identity.is_some() && used(&endpoint.id, *source))
        .map(|(endpoint, source)| {
            let remembered = endpoint.identity.clone();
            let path = std::path::PathBuf::from(&endpoint.path);
            (endpoint, source, std::thread::spawn(move || drives::check_in_time(remembered.as_ref(), &path)))
        })
        .collect();
    let mut issues = Vec::new();
    for (endpoint, source, check) in checks {
        let Some(remembered) = endpoint.identity.as_ref() else {
            continue;
        };
        if let Ok(DriveCheck::Different { found }) = check.join() {
            let code = if source { "drive-changed-source" } else { "drive-changed-destination" };
            let params = [
                ("path", endpoint.path.clone()),
                ("expected", remembered.name()),
                ("found", found.name()),
                ("endpoint", endpoint.id.clone()),
            ];
            issues.push(Issue::new(Severity::Error, code, &params, None));
        }
    }
    issues
}

fn report(link: &ResolvedLink, source_state: PathState, target_state: PathState, ready: bool) -> LinkReport {
    LinkReport {
        id: link.id.clone(),
        source: link.source.display().to_string(),
        target: link.target.display().to_string(),
        source_state,
        target_state,
        ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Endpoint, Link, Mode};

    fn task(mode: Mode, pairs: &[(&std::path::Path, &std::path::Path, &str)]) -> Task {
        let mut t = Task {
            name: "t".into(),
            mode,
            ..Task::default()
        };
        for (i, (s, d, sub)) in pairs.iter().enumerate() {
            t.sources.push(Endpoint {
                id: format!("s{i}"),
                path: s.display().to_string(),
                ..Default::default()
            });
            t.destinations.push(Endpoint {
                id: format!("d{i}"),
                path: d.display().to_string(),
                ..Default::default()
            });
            t.links.push(Link {
                id: format!("l{i}"),
                source_id: format!("s{i}"),
                destination_id: format!("d{i}"),
                subfolder: sub.to_string(),
                enabled: true,
            });
        }
        t
    }

    fn codes(t: &Task) -> Vec<String> {
        validate(t)
            .issues
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn another_drive_blocks_until_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut t = task(Mode::Mirror, &[(&a, &b, "")]);
        assert!(drives::learn_missing(&mut t));
        assert!(!codes(&t).iter().any(|c| c.starts_with("drive-changed")), "même disque : rien à signaler");
        // Un autre disque a pris la place de la destination.
        t.destinations[0].identity.as_mut().unwrap().id = "AUTRE-DISQUE".into();
        let report = validate(&t);
        assert!(!report.runnable);
        let issue = report.issues.iter().find(|i| i.code == "drive-changed-destination").expect("refus");
        assert_eq!(issue.params["endpoint"], "d0");
        // Après confirmation, l'identité est réapprise et l'opération redevient possible.
        t.destinations[0].identity = None;
        drives::learn_missing(&mut t);
        assert!(validate(&t).runnable);
    }

    #[test]
    fn same_and_nested_paths_block() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let nested = a.join("copie");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(codes(&task(Mode::Update, &[(&a, &a, "")])).contains(&"same-path".to_string()));
        assert!(codes(&task(Mode::Update, &[(&a, &nested, "")])).contains(&"target-in-source".to_string()));
        assert!(codes(&task(Mode::Mirror, &[(&nested, &a, "")])).contains(&"source-in-target".to_string()));
    }

    #[test]
    fn subfolder_can_create_a_hidden_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(a.join("x")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // La destination paraît distincte, le sous-dossier la ramène dans la source.
        assert!(codes(&task(Mode::Update, &[(&a, dir.path(), "a/x")])).contains(&"target-in-source".to_string()));
    }

    #[test]
    fn escaping_subfolder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert!(codes(&task(Mode::Update, &[(&a, &b, "../c")])).contains(&"subfolder".to_string()));
    }

    #[test]
    fn merging_needs_consent_and_is_forbidden_in_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        let mut t = task(Mode::Update, &[(&a, &c, ""), (&b, &c, "")]);
        assert!(codes(&t).contains(&"merge".to_string()));
        t.options.allow_merge = true;
        assert!(codes(&t).is_empty());
        t.mode = Mode::Mirror;
        assert!(codes(&t).contains(&"merge-mirror".to_string()));
    }

    #[test]
    fn mirror_parent_of_other_target_is_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        let t = task(Mode::Mirror, &[(&a, &c, ""), (&b, &c, "sous")]);
        assert!(codes(&t).contains(&"nested-targets-mirror".to_string()));
    }

    #[test]
    fn a_target_cannot_touch_another_source() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        let t = task(Mode::Update, &[(&a, &b, ""), (&b, &c, "")]);
        assert!(codes(&t).contains(&"target-touches-source".to_string()));
    }

    #[test]
    fn missing_disk_keeps_the_task_but_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        let gone = if cfg!(windows) {
            std::path::PathBuf::from(r"Q:\NonMonte")
        } else {
            std::path::PathBuf::from("/nonexistent-disk/x")
        };
        let t = task(Mode::Update, &[(&gone, &a, "")]);
        let report = validate(&t);
        assert!(!report.runnable);
        assert!(report.issues.iter().any(|i| i.code == "source-missing" && i.severity == Severity::Warning));
    }

    #[test]
    fn protected_folder_cannot_be_a_mirror_target() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        let home = std::path::PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap());
        assert!(codes(&task(Mode::Mirror, &[(&a, &home, "")])).contains(&"protected-target".to_string()));
    }

    #[test]
    fn clean_task_is_runnable() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        let report = validate(&task(Mode::Update, &[(&a, &b, "")]));
        assert!(report.runnable, "{:?}", report.issues);
        assert_eq!(report.links[0].target_state, PathState::WillBeCreated);
    }
}
