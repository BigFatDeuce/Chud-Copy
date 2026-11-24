//! Exécution d'une tâche, toujours déclenchée par un geste de l'utilisateur.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::{Control, Engine, EngineEvent, EngineStats, EntryKind, JobSpec, JobStatus, LogLevel};
use crate::errors::FriendlyError;
use crate::history::{self, Journal, LinkFile, LinkPhase, LinkRecord, RunError, RunRecord, RunStatus, Totals};
use crate::model::{ResolvedLink, Task};
use crate::plan::{job_for, link_volumes, Plan};
use crate::schedule::{run_jobs, Job};
use crate::store::Store;
use crate::validate::{self, Issue, Severity};
use crate::{attrs, fail, fail_text, new_id, now_ms, ui_text, volumes, Result};

const TICK: Duration = Duration::from_millis(250);
const CHECKPOINT: Duration = Duration::from_secs(5);
const MAX_RECORDED_ERRORS: usize = 5000;
const MAX_LIVE_ERRORS: usize = 200;

pub type Emit = Arc<dyn Fn(&RunRecord) + Send + Sync>;

struct Live {
    record: RunRecord,
    errors_total: u64,
    pause_started: Option<Instant>,
    paused_total: Duration,
    planned: HashMap<String, (u64, u64)>,
}

pub struct RunHandle {
    pub run_id: String,
    pub control: Control,
    finished: Arc<AtomicBool>,
    live: Arc<Mutex<Live>>,
}

impl RunHandle {
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> RunRecord {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).record.clone()
    }
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %Hh%M %S").to_string()
}

fn clock(journal_start: Instant) -> String {
    let seconds = journal_start.elapsed().as_secs();
    format!("{:02}:{:02}:{:02}", seconds / 3600, (seconds / 60) % 60, seconds % 60)
}

/// Pour un miroir, exige une analyse valable et renvoie les suppressions acceptées par liaison.
pub fn validated_deletions(task: &Task, plan: Option<&Arc<Plan>>) -> Result<HashMap<String, HashSet<String>>> {
    let mut validated = HashMap::new();
    if task.mode.removes_from_destination() {
        let Some(plan) = plan else {
            return fail_text("err.mirror-needs-analysis");
        };
        if plan.task_id != task.id || plan.task_updated_at != task.updated_at {
            return fail_text("err.task-changed");
        }
        for link in &plan.links {
            if link.failed {
                return fail_text("err.analysis-failed");
            }
            if link.unseen_files > 0 {
                return fail_text("err.source-unseen");
            }
            let deletions = link.entries.iter().filter(|e| e.kind == EntryKind::Delete).map(|e| e.rel.clone()).collect();
            validated.insert(link.link_id.clone(), deletions);
        }
    }
    Ok(validated)
}

/// Vérifie le contrat de sécurité puis lance l'exécution sur son propre fil.
pub fn start(engine: Arc<dyn Engine>, store: Store, task: Task, plan: Option<Arc<Plan>>, emit: Emit) -> Result<RunHandle> {
    start_then(engine, store, task, plan, emit, |_| {})
}

/// Comme `start`, et appelle `then` avec le statut final une fois l'exécution entièrement terminée.
pub fn start_then(
    engine: Arc<dyn Engine>,
    store: Store,
    task: Task,
    plan: Option<Arc<Plan>>,
    emit: Emit,
    then: impl FnOnce(RunStatus) + Send + 'static,
) -> Result<RunHandle> {
    let report = validate::validate(&task);
    if let Some(issue) = report.issues.iter().find(|i| i.severity == Severity::Error) {
        return fail(issue.text());
    }
    if !report.runnable {
        return fail_text("err.no-link-ready");
    }

    let validated = validated_deletions(&task, plan.as_ref())?;

    let run_id = new_id();
    let mut planned = HashMap::new();
    if let Some(plan) = &plan {
        if plan.task_id == task.id && plan.task_updated_at == task.updated_at {
            for link in &plan.links {
                planned.insert(link.link_id.clone(), (link.counts.transfer_bytes(), link.counts.transfer_files()));
            }
        }
    }

    let resolved: Vec<ResolvedLink> = task.resolved_links().into_iter().filter(|l| l.enabled).collect();
    let links = resolved
        .iter()
        .map(|link| {
            let state = report.links.iter().find(|r| r.id == link.id);
            let ready = state.is_some_and(|r| r.ready);
            let (bytes, files) = planned.get(&link.id).copied().unwrap_or((0, 0));
            LinkRecord {
                id: link.id.clone(),
                source: link.source.display().to_string(),
                target: link.target.display().to_string(),
                phase: if ready { LinkPhase::Waiting } else { LinkPhase::Skipped },
                total_bytes: bytes,
                total_transfers: files,
                message: if ready {
                    None
                } else {
                    report.issues.iter().find(|i| i.link_id.as_deref() == Some(link.id.as_str())).map(Issue::text)
                },
                ..LinkRecord::default()
            }
        })
        .collect();

    let record = RunRecord {
        id: run_id.clone(),
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        mode: task.mode,
        status: RunStatus::Running,
        started_at: now_ms(),
        links,
        ..RunRecord::default()
    };
    history::save(&store, &record)?;
    let journal = Arc::new(Journal::create(&store, &run_id)?);

    let live = Arc::new(Mutex::new(Live {
        record,
        errors_total: 0,
        pause_started: None,
        paused_total: Duration::ZERO,
        planned,
    }));
    let control = Control::default();
    let finished = Arc::new(AtomicBool::new(false));

    let context = Context {
        engine,
        store,
        task,
        journal,
        live: live.clone(),
        control: control.clone(),
        emit,
        validated,
        started: Instant::now(),
    };
    let done = finished.clone();
    let result = live.clone();
    let store = context.store.clone();
    std::thread::spawn(move || {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context.run(resolved))).is_err() {
            // Dernier filet : l'opération est close comme échouée plutôt que de rester « en cours ».
            let record = {
                let mut live = result.lock().unwrap_or_else(|e| e.into_inner());
                live.record.status = RunStatus::Failed;
                live.record.ended_at = Some(now_ms());
                live.record.clone()
            };
            let _ = history::save(&store, &record);
        }
        done.store(true, Ordering::SeqCst);
        let status = result.lock().unwrap_or_else(|e| e.into_inner()).record.status;
        then(status);
    });

    Ok(RunHandle {
        run_id,
        control,
        finished,
        live,
    })
}

struct Context {
    engine: Arc<dyn Engine>,
    store: Store,
    task: Task,
    journal: Arc<Journal>,
    live: Arc<Mutex<Live>>,
    control: Control,
    emit: Emit,
    /// Pour un miroir, les seuls fichiers que l'utilisateur a accepté de voir disparaître.
    validated: HashMap<String, HashSet<String>>,
    started: Instant,
}

impl Context {
    fn run(self, links: Vec<ResolvedLink>) {
        let this = Arc::new(self);
        let disks = volumes::all();
        let stamp = timestamp();

        this.journal.write(&format!("Tâche : {}", this.task.name));
        this.journal.write(&format!("Mode : {:?}", this.task.mode));

        let stop_ticker = Arc::new(AtomicBool::new(false));
        let ticker = {
            let this = this.clone();
            let stop = stop_ticker.clone();
            std::thread::spawn(move || this.tick_loop(&stop))
        };

        let jobs: Vec<Job<ResolvedLink>> = links
            .into_iter()
            .filter(|link| this.phase_of(&link.id) == LinkPhase::Waiting)
            .map(|link| Job {
                volumes: link_volumes(&link, &disks),
                item: link,
            })
            .collect();

        let control = this.control.clone();
        run_jobs(
            jobs,
            this.task.options.parallel_volumes,
            &|| control.is_cancelled(),
            &|link: ResolvedLink| {
                // Une erreur imprévue dans une liaison la marque en échec ; les autres continuent,
                // et l'opération se termine toujours.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| this.run_link(&link, &stamp)));
                if let Err(panic) = outcome {
                    let reason = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    this.fail_link(&link, "interne", &reason, &reason);
                }
            },
            &|link: ResolvedLink| {
                this.update_link(&link.id, |record| {
                    record.phase = LinkPhase::Cancelled;
                    record.status = Some(JobStatus::Cancelled);
                });
            },
        );

        stop_ticker.store(true, Ordering::SeqCst);
        let _ = ticker.join();
        this.finish();
    }

    fn phase_of(&self, id: &str) -> LinkPhase {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.record.links.iter().find(|l| l.id == id).map(|l| l.phase).unwrap_or(LinkPhase::Skipped)
    }

    fn update_link(&self, id: &str, change: impl FnOnce(&mut LinkRecord)) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(link) = live.record.links.iter_mut().find(|l| l.id == id) {
            change(link);
        }
    }

    fn push_errors(&self, link: &ResolvedLink, errors: &[FriendlyError]) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        for error in errors {
            live.errors_total += 1;
            if live.record.errors.len() < MAX_RECORDED_ERRORS {
                let entry = RunError {
                    link_id: link.id.clone(),
                    target: link.target.display().to_string(),
                    error: Some(error.clone()),
                };
                live.record.errors.push(entry);
            }
        }
    }

    fn run_link(&self, link: &ResolvedLink, stamp: &str) {
        self.update_link(&link.id, |record| record.phase = LinkPhase::Copying);
        let label = link
            .source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| link.source.display().to_string());
        self.journal.write(&format!(
            "{} [{label}] {} -> {}",
            clock(self.started),
            link.source.display(),
            link.target.display()
        ));

        let allowed = self.validated.get(&link.id);
        let job = job_for(&self.task, link, allowed.map(|set| set.len() as u64), stamp);

        // `--max-delete` limite un nombre, pas une identité : un fichier arrivé après l'analyse
        // pourrait prendre la place d'un fichier validé. L'état réel est donc recomparé juste
        // avant d'écrire, et le moindre écart arrête cette liaison sans y toucher.
        if let Some(allowed) = allowed {
            match self.deletions_are_validated(&job, allowed) {
                Ok(true) => {}
                Ok(false) if self.control.is_cancelled() => {
                    self.update_link(&link.id, |record| {
                        record.phase = LinkPhase::Cancelled;
                        record.status = Some(JobStatus::Cancelled);
                    });
                    return;
                }
                Ok(false) => {
                    let error = "--max-delete threshold reached: the destination changed since the validated analysis";
                    self.fail_link(link, &label, error, &ui_text("msg.destination-changed", &[]));
                    return;
                }
                Err(message) => {
                    self.fail_link(link, &label, &message, &message);
                    return;
                }
            }
        }

        let outcome = self.engine.transfer(&job, &self.control, &mut |event| match event {
            EngineEvent::Stats(stats) => self.apply_stats(&link.id, &stats),
            EngineEvent::Log { level, text } => {
                let tag = match level {
                    LogLevel::Error => "ERREUR",
                    LogLevel::Notice => "NOTE",
                    LogLevel::Info => "INFO",
                };
                self.journal.write(&format!("{} [{label}] {tag} {text}", clock(self.started)));
            }
        });

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(e) => {
                let message = e.to_string();
                self.fail_link(link, &label, &message, &message);
                return;
            }
        };

        self.push_errors(link, &outcome.errors);
        self.apply_stats(&link.id, &outcome.stats);
        let mut status = outcome.status;

        if self.task.options.preserve_metadata {
            let failures = attrs::restore(&link.source, &link.target, &outcome.written);
            if failures > 0 {
                self.journal.write(&format!(
                    "{} [{label}] NOTE Attributs non restaurés pour {failures} fichier(s), que ce disque ne prend pas en charge",
                    clock(self.started)
                ));
            }
        }

        if self.task.options.verify_after && status != JobStatus::Cancelled && !outcome.written.is_empty() {
            let total = outcome.written.len() as u64;
            self.update_link(&link.id, |record| {
                record.phase = LinkPhase::Verifying;
                record.verify_total = total;
                record.current.clear();
            });
            self.journal
                .write(&format!("{} [{label}] Contrôle de {total} fichier(s) après transfert", clock(self.started)));
            let verified = self.engine.verify(&job, &outcome.written, &self.control, &mut |stats: &EngineStats| {
                self.update_link(&link.id, |record| record.verify_checked = stats.checks);
            });
            match verified {
                Ok(result) => {
                    self.update_link(&link.id, |record| {
                        record.verify_checked = result.checked;
                        record.verify_failures = result.failures.len() as u64;
                    });
                    self.push_errors(link, &result.failures);
                    if result.cancelled {
                        status = JobStatus::Cancelled;
                    } else if !result.failures.is_empty() {
                        status = JobStatus::CompletedWithErrors;
                    }
                }
                Err(e) => {
                    self.push_errors(link, &[crate::errors::humanize(None, &e.to_string())]);
                    status = JobStatus::CompletedWithErrors;
                }
            }
        }

        self.journal.write(&format!("{} [{label}] Terminé : {status:?}", clock(self.started)));
        self.update_link(&link.id, |record| {
            record.status = Some(status);
            record.phase = match status {
                JobStatus::Completed | JobStatus::CompletedWithErrors => LinkPhase::Done,
                JobStatus::Failed => LinkPhase::Failed,
                JobStatus::Cancelled => LinkPhase::Cancelled,
            };
            record.current.clear();
            record.speed = 0.0;
            record.eta = None;
        });
    }

    fn deletions_are_validated(&self, job: &JobSpec, allowed: &HashSet<String>) -> std::result::Result<bool, String> {
        let current = self.engine.analyze(job, &self.control, &mut |_| {}).map_err(|e| e.to_string())?;
        if current.cancelled {
            return Ok(false);
        }
        if current.failed {
            let reason = current.errors.first().map(|e| e.detail.clone());
            return Err(reason.unwrap_or_else(|| ui_text("msg.precheck-failed", &[])));
        }
        Ok(current.entries.iter().filter(|e| e.kind == EntryKind::Delete).all(|e| allowed.contains(&e.rel)))
    }

    fn fail_link(&self, link: &ResolvedLink, label: &str, error: &str, message: &str) {
        self.journal.write(&format!("{} [{label}] ERREUR {error}", clock(self.started)));
        self.push_errors(link, &[crate::errors::humanize(None, error)]);
        self.update_link(&link.id, |record| {
            record.phase = LinkPhase::Failed;
            record.status = Some(JobStatus::Failed);
            record.message = Some(message.to_string());
            record.current.clear();
        });
    }

    fn apply_stats(&self, id: &str, stats: &EngineStats) {
        self.update_link(id, |record| {
            record.bytes = stats.bytes;
            // Le moteur fait foi dès qu'il a établi son total.
            if stats.total_bytes > 0 || stats.total_transfers > 0 {
                record.total_bytes = stats.total_bytes;
                record.total_transfers = stats.total_transfers;
            }
            record.transfers = stats.transfers;
            record.checks = stats.checks;
            record.total_checks = stats.total_checks;
            record.deletes = stats.deletes;
            record.errors = stats.errors;
            record.speed = stats.speed;
            record.eta = stats.eta;
            record.elapsed = stats.elapsed;
            record.average_speed = if stats.transfer_time > 0.0 {
                stats.bytes as f64 / stats.transfer_time
            } else {
                0.0
            };
            record.current = stats.transferring.iter().map(LinkFile::from).collect();
        });
    }

    fn refresh(&self) -> RunRecord {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let paused = self.control.is_paused();
        match (paused, live.pause_started) {
            (true, None) => live.pause_started = Some(Instant::now()),
            (false, Some(since)) => {
                live.paused_total += since.elapsed();
                live.pause_started = None;
            }
            _ => {}
        }
        let paused_now = live.pause_started.map(|s| s.elapsed()).unwrap_or_default();
        let active_time = self.started.elapsed().saturating_sub(live.paused_total + paused_now);

        live.record.status = if self.control.is_cancelled() {
            RunStatus::Cancelling
        } else if paused {
            RunStatus::Paused
        } else {
            RunStatus::Running
        };
        live.record.elapsed = active_time.as_secs_f64();
        live.record.errors_total = live.errors_total;
        let planned = live.planned.clone();
        live.record.totals = totals(&live.record.links, &planned, live.record.elapsed, paused);
        let mut out = live.record.clone();
        if out.errors.len() > MAX_LIVE_ERRORS {
            out.errors.drain(..out.errors.len() - MAX_LIVE_ERRORS);
        }
        out
    }

    fn tick_loop(&self, stop: &AtomicBool) {
        let mut last_checkpoint = Instant::now();
        while !stop.load(Ordering::SeqCst) {
            let snapshot = self.refresh();
            (self.emit)(&snapshot);
            if last_checkpoint.elapsed() >= CHECKPOINT {
                // Le rapport survit à une coupure : la dernière image reste dans l'historique.
                // Copie prise sous le verrou, écriture faite sans : le suivi de la copie n'attend pas le disque.
                let record = self.live.lock().unwrap_or_else(|e| e.into_inner()).record.clone();
                let _ = history::save(&self.store, &record);
                self.journal.flush();
                last_checkpoint = Instant::now();
            }
            std::thread::sleep(TICK);
        }
    }

    fn finish(&self) {
        self.refresh();
        let record = {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            let links = &live.record.links;
            let done = links.iter().filter(|l| l.status == Some(JobStatus::Completed)).count();
            let partial = links.iter().filter(|l| l.status == Some(JobStatus::CompletedWithErrors)).count();
            let cancelled = links.iter().any(|l| l.status == Some(JobStatus::Cancelled));
            let status = if cancelled {
                RunStatus::Cancelled
            } else if done == links.len() {
                RunStatus::Completed
            } else if done + partial > 0 {
                RunStatus::CompletedWithErrors
            } else {
                RunStatus::Failed
            };
            live.record.status = status;
            live.record.ended_at = Some(now_ms());
            live.record.totals.speed = 0.0;
            live.record.totals.eta = None;
            live.record.clone()
        };
        self.journal.write(&format!("Fin de l'opération : {:?}", record.status));
        self.journal.flush();
        let _ = history::save(&self.store, &record);
        let _ = history::remember(&self.store, std::slice::from_ref(&record));
        (self.emit)(&record);
    }
}

fn totals(links: &[LinkRecord], planned: &HashMap<String, (u64, u64)>, elapsed: f64, paused: bool) -> Totals {
    let mut out = Totals {
        total_known: true,
        ..Totals::default()
    };
    let mut transfer_time = 0.0_f64;
    for link in links {
        if link.phase == LinkPhase::Skipped {
            continue;
        }
        out.bytes += link.bytes;
        out.transfers += link.transfers;
        out.deletes += link.deletes;
        out.errors += link.errors;
        out.speed += link.speed;
        if link.phase == LinkPhase::Waiting && !planned.contains_key(&link.id) {
            out.total_known = false;
        }
        out.total_bytes += link.total_bytes;
        out.total_transfers += link.total_transfers;
        transfer_time = transfer_time.max(link.elapsed);
    }
    let reference = if elapsed > 0.0 { elapsed } else { transfer_time };
    out.average_speed = if reference > 0.0 { out.bytes as f64 / reference } else { 0.0 };
    if out.total_known && out.speed > 0.0 && !paused {
        out.eta = Some(out.total_bytes.saturating_sub(out.bytes) as f64 / out.speed);
    }
    out
}
