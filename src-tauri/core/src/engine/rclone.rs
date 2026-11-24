//! Implémentation du moteur par rclone, embarqué avec l'application.
//!
//! Chaque liaison lance son propre processus : pas de démon, pas de port permanent, et l'échec
//! d'une destination reste isolé. Le port de contrôle local n'existe que le temps de l'opération,
//! protégé par un secret aléatoire, et sert uniquement à demander un arrêt propre.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;

use super::{
    Analysis, Control, Engine, EngineEvent, EngineStats, EntryKind, FileProgress, JobSpec, JobStatus, LogLevel, PlanEntry, TransferOutcome, VerifyOutcome,
};
use crate::errors::{humanize, FriendlyError};
use crate::filters::{self, ARCHIVE_DIR};
use crate::model::{Compare, Mode, Symlinks};
use crate::{fail, fail_text, new_id, proc, ui_text, Result};

/// Délai laissé au moteur pour nettoyer ses fichiers partiels avant de l'arrêter de force.
const STOP_GRACE: Duration = Duration::from_secs(8);
const STATS_INTERVAL: &str = "500ms";
const NULL_CONFIG: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

pub struct Rclone {
    pub binary: PathBuf,
    pub work_dir: PathBuf,
}

#[derive(PartialEq, Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

struct Finished {
    code: Option<i32>,
    cancelled: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LogLine {
    level: String,
    msg: String,
    object: Option<serde_json::Value>,
    size: Option<i64>,
    skipped: Option<String>,
    stats: Option<RawStats>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RawStats {
    bytes: u64,
    total_bytes: u64,
    speed: f64,
    eta: Option<f64>,
    transfers: u64,
    total_transfers: u64,
    checks: u64,
    total_checks: u64,
    deletes: u64,
    errors: u64,
    listed: u64,
    elapsed_time: f64,
    transfer_time: f64,
    transferring: Vec<RawTransfer>,
    checking: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RawTransfer {
    name: String,
    size: i64,
    bytes: i64,
    percentage: u32,
    speed_avg: f64,
}

impl From<RawStats> for EngineStats {
    fn from(raw: RawStats) -> Self {
        EngineStats {
            bytes: raw.bytes,
            total_bytes: raw.total_bytes,
            speed: raw.speed,
            eta: raw.eta,
            transfers: raw.transfers,
            total_transfers: raw.total_transfers,
            checks: raw.checks,
            total_checks: raw.total_checks,
            deletes: raw.deletes,
            errors: raw.errors,
            listed: raw.listed,
            elapsed: raw.elapsed_time,
            transfer_time: raw.transfer_time,
            checking: raw.checking,
            transferring: raw
                .transferring
                .into_iter()
                .map(|t| FileProgress {
                    name: t.name,
                    size: t.size.max(0) as u64,
                    bytes: t.bytes.max(0) as u64,
                    percentage: t.percentage,
                    speed: t.speed_avg,
                })
                .collect(),
        }
    }
}

/// Fichier temporaire de travail, retiré quoi qu'il arrive.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn object_name(line: &LogLine) -> Option<String> {
    line.object.as_ref()?.as_str().map(str::to_string)
}

fn is_partial_leftover(rel: &str) -> bool {
    let Some(stem) = rel.strip_suffix(".partial") else { return false };
    let Some((_, tag)) = stem.rsplit_once('.') else { return false };
    tag.len() == 8 && tag.chars().all(|c| c.is_ascii_hexdigit())
}

/// Messages de déroulement du moteur qui n'apprennent rien à l'utilisateur.
fn is_noise(message: &str) -> bool {
    message.starts_with("Attempt ")
        || message.starts_with("Fatal error received")
        || message.starts_with("Cancelling sync due to fatal error")
        || message.starts_with("Failed to sync")
        || message.starts_with("Failed to copy with")
}

/// Vrai si le message annonce une nouvelle tentative : les erreurs précédentes ne sont plus définitives.
fn retry_follows(message: &str) -> bool {
    let Some(rest) = message.strip_prefix("Attempt ") else { return false };
    let Some((progress, _)) = rest.split_once(' ') else { return false };
    let Some((current, total)) = progress.split_once('/') else { return false };
    matches!((current.parse::<u32>(), total.parse::<u32>()), (Ok(c), Ok(t)) if c < t)
}

impl Rclone {
    pub fn new(binary: PathBuf, work_dir: PathBuf) -> Self {
        Self { binary, work_dir }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        // Un RCLONE_* de l'environnement ne doit jamais changer le sens d'une opération.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().to_uppercase().starts_with("RCLONE_") {
                command.env_remove(key);
            }
        }
        // Depuis Go 1.23, un fichier géré par un service cloud (MEGA, Nextcloud, OneDrive) n'est plus
        // un fichier ordinaire pour Go, et rclone l'ignore sans aucun message. Ce réglage rétablit
        // le comportement attendu tout en laissant les vrais liens et jonctions traités comme tels.
        if cfg!(windows) {
            command.env("GODEBUG", "winsymlink=0");
        }
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        proc::prepare(&mut command);
        command
    }

    fn write_filter(&self, job: &JobSpec) -> Result<TempFile> {
        std::fs::create_dir_all(&self.work_dir)?;
        let path = self.work_dir.join(format!("{}.filter", new_id()));
        let rules = filters::rules(&job.filters, &job.options, &job.source);
        std::fs::write(&path, rules.join("\n"))?;
        Ok(TempFile(path))
    }

    fn write_file_list(&self, files: &[String]) -> Result<TempFile> {
        std::fs::create_dir_all(&self.work_dir)?;
        let path = self.work_dir.join(format!("{}.files", new_id()));
        std::fs::write(&path, files.join("\n"))?;
        Ok(TempFile(path))
    }

    /// Lance le moteur et applique les consignes de pause et d'arrêt tant qu'il tourne.
    fn drive(&self, mut command: Command, control: &Control, on_line: &mut dyn FnMut(Stream, &str)) -> Result<Finished> {
        let secret = new_id();
        command.env("RCLONE_RC_USER", "chud").env("RCLONE_RC_PASS", &secret);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return fail(ui_text(
                    "err.engine-start",
                    &[("path", self.binary.display().to_string()), ("detail", e.to_string())],
                ))
            }
        };
        proc::adopt(&child);

        let (tx, rx) = mpsc::channel::<(Stream, Vec<u8>)>();
        spawn_reader(child.stdout.take(), Stream::Stdout, tx.clone());
        spawn_reader(child.stderr.take(), Stream::Stderr, tx);

        let mut port: Option<u16> = None;
        let mut suspended = false;
        let mut stop_at: Option<Instant> = None;
        let mut quit_sent = false;

        loop {
            match rx.recv_timeout(Duration::from_millis(80)) {
                Ok((stream, bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let text = text.trim_end_matches(['\r', '\n']);
                    if port.is_none() && stream == Stream::Stderr {
                        port = parse_control_port(text);
                    }
                    on_line(stream, text);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if control.is_cancelled() {
                if suspended {
                    suspended = !proc::resume(&child);
                }
                let since = *stop_at.get_or_insert_with(Instant::now);
                if !quit_sent {
                    if let Some(port) = port {
                        quit_sent = ask_to_quit(port, &secret);
                    }
                }
                if since.elapsed() > STOP_GRACE {
                    let _ = child.kill();
                }
            } else if control.is_paused() && !suspended {
                suspended = proc::suspend(&child);
            } else if !control.is_paused() && suspended {
                suspended = !proc::resume(&child);
            }
        }

        let status = child.wait()?;
        Ok(Finished {
            code: status.code(),
            cancelled: control.is_cancelled(),
        })
    }

    fn run_to_string(&self, args: &[&str]) -> Result<String> {
        let mut command = self.command();
        command.args(args);
        let output = command.output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

fn spawn_reader(pipe: Option<impl Read + Send + 'static>, stream: Stream, tx: mpsc::Sender<(Stream, Vec<u8>)>) {
    let Some(pipe) = pipe else { return };
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        loop {
            let mut buffer = Vec::new();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send((stream, buffer)).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn parse_control_port(line: &str) -> Option<u16> {
    let (_, rest) = line.split_once("Serving remote control on http://127.0.0.1:")?;
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

fn ask_to_quit(port: u16, secret: &str) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(800)) else {
        return false;
    };
    let token = base64::engine::general_purpose::STANDARD.encode(format!("chud:{secret}"));
    let body = r#"{"exitCode":130}"#;
    let request = format!(
        "POST /core/quit HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).is_ok()
}

fn verb(mode: Mode) -> &'static str {
    if mode.removes_from_destination() {
        "sync"
    } else {
        "copy"
    }
}

fn link_flag(symlinks: Symlinks) -> &'static str {
    match symlinks {
        Symlinks::Skip => "--skip-links",
        Symlinks::Follow => "--copy-links",
        Symlinks::Preserve => "--links",
    }
}

fn push(args: &mut Vec<OsString>, values: &[&str]) {
    args.extend(values.iter().map(OsString::from));
}

/// Options communes à toutes les commandes d'une liaison.
fn common_flags(job: &JobSpec) -> Vec<OsString> {
    let mut args = Vec::new();
    // Les vérifications (lister, comparer) attendent le serveur à chaque dossier sur un partage
    // réseau : davantage de vérifications simultanées y masquent cette attente.
    let network = crate::drives::is_network_path(&job.source) || crate::drives::is_network_path(&job.target);
    let transfers = job.options.transfers.max(1);
    let checkers = if network { (transfers * 4).max(16) } else { transfers * 2 }.to_string();
    push(
        &mut args,
        &[
            "--config",
            NULL_CONFIG,
            "--use-json-log",
            "--log-level",
            "INFO",
            "--stats",
            STATS_INTERVAL,
            "--stats-log-level",
            "NOTICE",
            "--rc",
            "--rc-addr",
            "127.0.0.1:0",
            "--checkers",
            &checkers,
            link_flag(job.options.symlinks),
        ],
    );
    if cfg!(windows) {
        push(&mut args, &["--ignore-case"]);
    }
    args
}

/// Arguments décrivant le transfert lui-même, partagés par l'analyse et l'exécution :
/// c'est ce qui garantit qu'une analyse prédit bien ce que l'exécution fera.
fn transfer_flags(job: &JobSpec, filter_file: &Path) -> Vec<OsString> {
    let mut args = vec![
        OsString::from(verb(job.mode)),
        job.source.clone().into_os_string(),
        job.target.clone().into_os_string(),
    ];
    args.extend(common_flags(job));

    let transfers = job.options.transfers.max(1).to_string();
    let retries = job.options.retries.max(1).to_string();
    push(
        &mut args,
        &[
            "--transfers",
            &transfers,
            "--retries",
            &retries,
            "--retries-sleep",
            "2s",
            // Tout est connu avant la première écriture : le total affiché est réel dès le début.
            "--check-first",
        ],
    );

    args.push("--filter-from".into());
    args.push(filter_file.as_os_str().to_os_string());

    match job.mode {
        Mode::Full => push(&mut args, &["--ignore-times"]),
        Mode::AddOnly => push(&mut args, &["--ignore-existing"]),
        Mode::MirrorArchive => {
            args.push("--backup-dir".into());
            args.push(job.target.join(ARCHIVE_DIR).join(&job.archive_stamp).into_os_string());
        }
        Mode::Update | Mode::Mirror => {}
    }

    match job.options.compare {
        // MEGA, Nextcloud, FAT ou exFAT arrondissent les dates. Sans tolérance, le moindre écart
        // sous la seconde force la relecture complète du fichier des deux côtés.
        Compare::SizeAndTime => push(&mut args, &["--modify-window", "1s"]),
        Compare::Checksum => push(&mut args, &["--checksum"]),
        Compare::SizeOnly => push(&mut args, &["--size-only"]),
    }
    if job.options.skip_newer && job.mode == Mode::Update {
        push(&mut args, &["--update"]);
    }
    if job.options.empty_dirs {
        push(&mut args, &["--create-empty-src-dirs"]);
    }
    if !job.options.multi_thread {
        // Plusieurs flux sur un même fichier font sauter la tête d'un disque mécanique.
        push(&mut args, &["--multi-thread-streams", "0"]);
    }

    if let Some(size) = job.filters.min_size {
        push(&mut args, &["--min-size", &format!("{size}b")]);
    }
    if let Some(size) = job.filters.max_size {
        push(&mut args, &["--max-size", &format!("{size}b")]);
    }
    if let Some(date) = &job.filters.newer_than {
        push(&mut args, &["--max-age", date]);
    }
    if let Some(date) = &job.filters.older_than {
        push(&mut args, &["--min-age", date]);
    }
    args
}

impl Engine for Rclone {
    fn name(&self) -> String {
        "rclone".to_string()
    }

    fn version(&self) -> Result<String> {
        let output = self.run_to_string(&["version"])?;
        match output.lines().next() {
            Some(line) => Ok(line.trim().to_string()),
            None => fail_text("err.engine-silent"),
        }
    }

    fn analyze(&self, job: &JobSpec, control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> Result<Analysis> {
        let filter = self.write_filter(job)?;
        let mut command = self.command();
        command.args(transfer_flags(job, &filter.0));
        command.args(["--dry-run", "--combined", "-"]);

        let removes = job.mode.removes_from_destination();
        let mut entries: Vec<PlanEntry> = Vec::new();
        let mut sizes: HashMap<String, u64> = HashMap::new();
        let mut errors: Vec<FriendlyError> = Vec::new();

        let finished = self.drive(command, control, &mut |stream, line| match stream {
            Stream::Stdout => {
                let Some((symbol, rel)) = line.split_once(' ') else { return };
                let kind = match symbol {
                    "+" => EntryKind::Copy,
                    "*" => EntryKind::Update,
                    "=" => EntryKind::Same,
                    "!" => EntryKind::Error,
                    "-" if removes => EntryKind::Delete,
                    "-" if is_partial_leftover(rel) => EntryKind::Incomplete,
                    "-" => EntryKind::Extra,
                    _ => return,
                };
                entries.push(PlanEntry {
                    rel: rel.to_string(),
                    kind,
                    size: None,
                    note: None,
                });
            }
            Stream::Stderr => {
                let Ok(parsed) = serde_json::from_str::<LogLine>(line) else { return };
                if let Some(stats) = parsed.stats {
                    progress(&stats.into());
                    return;
                }
                if parsed.skipped.is_some() {
                    if let (Some(object), Some(size)) = (object_name(&parsed), parsed.size) {
                        sizes.insert(object, size.max(0) as u64);
                    }
                    return;
                }
                if parsed.level == "error" && !is_noise(&parsed.msg) {
                    errors.push(humanize(object_name(&parsed).as_deref(), &parsed.msg));
                }
            }
        })?;

        for entry in &mut entries {
            entry.size = sizes.get(&entry.rel).copied();
        }

        // Le moteur ne décrit pas ce que les filtres écartent : la liste complète de la source
        // permet de le retrouver par différence.
        if !finished.cancelled && (job.filters.restricts_files() || !job.options.include_hidden) {
            let seen: HashSet<&str> = entries
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::Copy | EntryKind::Update | EntryKind::Same | EntryKind::Error))
                .map(|e| e.rel.as_str())
                .collect();
            let mut ignored = Vec::new();
            let mut command = self.command();
            command
                .arg("lsf")
                .arg(&job.source)
                .args(common_flags(job))
                .args(["-R", "--files-only", "--format", "sp", "--separator", "\t"]);
            self.drive(command, control, &mut |stream, line| {
                if stream != Stream::Stdout {
                    return;
                }
                let Some((size, rel)) = line.split_once('\t') else { return };
                if !seen.contains(rel) {
                    ignored.push(PlanEntry {
                        rel: rel.to_string(),
                        kind: EntryKind::Ignored,
                        size: size.parse().ok(),
                        note: None,
                    });
                }
            })?;
            entries.append(&mut ignored);
        }

        let failed = !matches!(finished.code, Some(0)) && !finished.cancelled;
        if failed && errors.is_empty() {
            errors.push(humanize(None, &format!("analysis ended with code {:?}", finished.code)));
        }
        Ok(Analysis {
            entries,
            errors,
            cancelled: finished.cancelled,
            failed,
        })
    }

    fn transfer(&self, job: &JobSpec, control: &Control, events: &mut dyn FnMut(EngineEvent)) -> Result<TransferOutcome> {
        let filter = self.write_filter(job)?;
        let mut command = self.command();
        command.args(transfer_flags(job, &filter.0));
        if job.mode.removes_from_destination() {
            let Some(limit) = job.max_delete else {
                return fail_text("err.mirror-needs-analysis");
            };
            command.args(["--max-delete", &limit.to_string()]);
        }

        let mut stats = EngineStats::default();
        let mut errors: Vec<FriendlyError> = Vec::new();
        let mut written: Vec<String> = Vec::new();

        let finished = self.drive(command, control, &mut |stream, line| {
            if stream != Stream::Stderr {
                return;
            }
            let Ok(parsed) = serde_json::from_str::<LogLine>(line) else {
                if !line.trim().is_empty() {
                    events(EngineEvent::Log {
                        level: LogLevel::Notice,
                        text: line.to_string(),
                    });
                }
                return;
            };
            if let Some(raw) = parsed.stats {
                stats = raw.into();
                events(EngineEvent::Stats(stats.clone()));
                return;
            }
            let object = object_name(&parsed);
            if parsed.msg.starts_with("Copied") {
                if let Some(rel) = &object {
                    written.push(rel.clone());
                }
            }
            if retry_follows(&parsed.msg) {
                errors.clear();
            }
            let level = match parsed.level.as_str() {
                "error" | "critical" => LogLevel::Error,
                "notice" | "warning" => LogLevel::Notice,
                _ => LogLevel::Info,
            };
            if level == LogLevel::Error && !is_noise(&parsed.msg) {
                errors.push(humanize(object.as_deref(), &parsed.msg));
            }
            let text = match object {
                Some(rel) => format!("{} : {rel}", parsed.msg.trim()),
                None => parsed.msg.trim().to_string(),
            };
            events(EngineEvent::Log { level, text });
        })?;

        let status = if finished.cancelled {
            JobStatus::Cancelled
        } else if finished.code == Some(0) {
            JobStatus::Completed
        } else if stats.transfers > 0 || stats.deletes > 0 {
            JobStatus::CompletedWithErrors
        } else {
            JobStatus::Failed
        };
        if status != JobStatus::Completed && status != JobStatus::Cancelled && errors.is_empty() {
            errors.push(humanize(None, &format!("engine exited with code {:?}", finished.code)));
        }
        Ok(TransferOutcome {
            status,
            stats,
            errors,
            written,
        })
    }

    fn verify(&self, job: &JobSpec, files: &[String], control: &Control, progress: &mut dyn FnMut(&EngineStats)) -> Result<VerifyOutcome> {
        if files.is_empty() {
            return Ok(VerifyOutcome::default());
        }
        let list = self.write_file_list(files)?;
        let mut command = self.command();
        command.arg("check").arg(&job.source).arg(&job.target).args(common_flags(job));
        command.args(["--one-way", "--combined", "-"]);
        command.arg("--files-from-raw").arg(&list.0);

        let mut outcome = VerifyOutcome::default();
        let finished = self.drive(command, control, &mut |stream, line| match stream {
            Stream::Stdout => {
                let Some((symbol, rel)) = line.split_once(' ') else { return };
                match symbol {
                    "=" => outcome.checked += 1,
                    "*" => outcome.failures.push(humanize(Some(rel), "hashes differ: corrupted on transfer")),
                    "+" => outcome
                        .failures
                        .push(humanize(Some(rel), "directory not found: missing from the destination after copy")),
                    "!" => outcome.failures.push(humanize(Some(rel), "i/o error during verification")),
                    _ => {}
                }
            }
            Stream::Stderr => {
                if let Ok(parsed) = serde_json::from_str::<LogLine>(line) {
                    if let Some(stats) = parsed.stats {
                        progress(&stats.into());
                    }
                }
            }
        })?;
        outcome.cancelled = finished.cancelled;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Filters, Options};

    fn job(mode: Mode) -> JobSpec {
        JobSpec {
            id: "x".into(),
            source: PathBuf::from("/src"),
            target: PathBuf::from("/dst"),
            mode,
            options: Options::default(),
            filters: Filters::default(),
            max_delete: None,
            archive_stamp: "stamp".into(),
        }
    }

    fn flags(job: &JobSpec) -> Vec<String> {
        transfer_flags(job, Path::new("f")).iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn only_mirrors_use_sync() {
        for (mode, expected) in [
            (Mode::Update, "copy"),
            (Mode::Full, "copy"),
            (Mode::AddOnly, "copy"),
            (Mode::Mirror, "sync"),
            (Mode::MirrorArchive, "sync"),
        ] {
            assert_eq!(flags(&job(mode))[0], expected);
        }
    }

    #[test]
    fn modes_map_to_the_expected_comparison() {
        assert!(flags(&job(Mode::Full)).contains(&"--ignore-times".to_string()));
        assert!(flags(&job(Mode::AddOnly)).contains(&"--ignore-existing".to_string()));
        assert!(flags(&job(Mode::MirrorArchive)).contains(&"--backup-dir".to_string()));
        assert!(!flags(&job(Mode::Mirror)).contains(&"--backup-dir".to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn cloud_managed_files_stay_visible_to_the_engine() {
        let engine = Rclone::new(PathBuf::from("rclone"), std::env::temp_dir());
        let command = engine.command();
        let setting = command.get_envs().find(|(key, _)| *key == "GODEBUG").and_then(|(_, value)| value);
        assert_eq!(setting, Some(std::ffi::OsStr::new("winsymlink=0")));
    }

    #[test]
    fn engine_metadata_stays_off_because_it_can_fail_a_copy() {
        for mode in [Mode::Update, Mode::Full, Mode::Mirror] {
            assert!(!flags(&job(mode)).iter().any(|f| f == "--metadata" || f == "-M"));
        }
    }

    #[test]
    fn dates_tolerate_the_rounding_of_cloud_services() {
        assert!(flags(&job(Mode::Update)).contains(&"--modify-window".to_string()));
        let mut exact = job(Mode::Update);
        exact.options.compare = Compare::Checksum;
        assert!(!flags(&exact).contains(&"--modify-window".to_string()));
    }

    #[test]
    fn excluded_destination_files_are_never_deleted() {
        for mode in [Mode::Update, Mode::Mirror, Mode::MirrorArchive] {
            assert!(!flags(&job(mode)).iter().any(|f| f == "--delete-excluded"));
        }
    }

    #[test]
    fn rejects_mirror_without_a_validated_limit() {
        let engine = Rclone::new(PathBuf::from("rclone-absent"), std::env::temp_dir());
        let result = engine.transfer(&job(Mode::Mirror), &Control::default(), &mut |_| {});
        assert!(result.is_err());
    }

    #[test]
    fn partial_leftovers_are_recognised() {
        assert!(is_partial_leftover("sous/gros.bin.2cc16515.partial"));
        assert!(!is_partial_leftover("rapport.partial"));
        assert!(!is_partial_leftover("gros.bin"));
    }

    #[test]
    fn retry_detection_reads_the_attempt_counter() {
        assert!(retry_follows("Attempt 1/3 failed with 1 errors and: x"));
        assert!(!retry_follows("Attempt 3/3 failed with 1 errors and: x"));
        assert!(!retry_follows("Copied (new)"));
    }

    #[test]
    fn control_port_is_read_from_the_log() {
        let line = r#"{"msg":"Serving remote control on http://127.0.0.1:53917/"}"#;
        assert_eq!(parse_control_port(line), Some(53917));
    }
}
