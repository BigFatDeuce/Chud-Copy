//! Testeur réseau et NAS : vérifie qu'un dossier partagé (SMB, NFS…) est joignable, mesure la
//! latence du serveur et du partage, puis les débits d'écriture et de lecture.
//!
//! La lecture se fait sur des fichiers déjà présents, sans les modifier. L'écriture n'a lieu que
//! si l'utilisateur la demande : un dossier de test caché est créé dans le partage, rempli,
//! relu pour vérifier les données, puis vidé et retiré. Seuls les fichiers créés par le test
//! sont supprimés, un par un ; rien d'autre n'est jamais touché.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::engine::Control;
use crate::tools::ToolProgress;
use crate::{fail_text, Result};

const TEST_DIR_PREFIX: &str = ".chudcopy-network-test-";
const CHUNK: usize = 1 << 20;
const SMALL_FILES: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkOptions {
    pub write_test: bool,
    /// Volume écrit puis relu, et volume lu au plus sur les fichiers existants.
    pub size: u64,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            write_test: false,
            size: 256 << 20,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Latency {
    /// Mesures en millisecondes.
    pub samples: Vec<f64>,
    pub failures: u32,
    pub min: f64,
    pub average: f64,
    pub max: f64,
    /// Écart moyen entre deux mesures successives.
    pub jitter: f64,
}

impl Latency {
    fn from(samples: Vec<f64>, failures: u32) -> Self {
        if samples.is_empty() {
            return Self { failures, ..Self::default() };
        }
        let min = samples.iter().copied().fold(f64::MAX, f64::min);
        let max = samples.iter().copied().fold(0.0, f64::max);
        let average = samples.iter().sum::<f64>() / samples.len() as f64;
        let jitter = if samples.len() > 1 {
            samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64
        } else {
            0.0
        };
        Self {
            samples,
            failures,
            min,
            average,
            max,
            jitter,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub bytes: u64,
    pub seconds: f64,
    /// Octets par seconde.
    pub speed: f64,
    pub samples: Vec<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkReport {
    pub path: String,
    /// « smb », « nfs », « sftp », « network » ou « local ».
    pub kind: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub address: Option<String>,
    pub resolve_ms: Option<f64>,
    /// Établissement d'une connexion TCP vers le service de partage.
    pub connect: Option<Latency>,
    pub reachable: bool,
    pub readable: bool,
    pub entries: u64,
    pub list_ms: f64,
    /// Temps de réponse du partage à une simple consultation d'informations.
    pub metadata: Latency,
    pub read: Option<Transfer>,
    /// Vrai si la lecture a dû se rabattre sur le fichier de test, peut-être servi par le cache.
    pub read_from_test: bool,
    pub writable: Option<bool>,
    pub write: Option<Transfer>,
    /// Création, écriture et suppression de petits fichiers, par seconde.
    pub small_files_per_second: Option<f64>,
    /// Les données relues sont-elles identiques à celles écrites ?
    pub verified: Option<bool>,
    /// Les fichiers de test ont-ils tous été retirés ?
    pub cleaned: Option<bool>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub elapsed: f64,
}

/// Le serveur et le type de partage derrière un chemin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub kind: &'static str,
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// « \\nas\partage », « //nas/partage » ou « nas:/export » → nom du serveur.
pub fn host_of(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("\\\\").or_else(|| trimmed.strip_prefix("//")) {
        let host = rest.split(['\\', '/']).next()?.trim();
        return (!host.is_empty()).then(|| host.split('@').next_back().unwrap_or(host).to_string());
    }
    if let Some((host, _)) = trimmed.split_once(":/") {
        let host = host.rsplit('@').next().unwrap_or(host);
        return (!host.is_empty() && !host.contains('/')).then(|| host.trim_matches(['[', ']']).to_string());
    }
    None
}

/// Le serveur d'un montage GVFS : « …/gvfs/smb-share:server=nas,share=photos/… ».
pub fn gvfs_share(path: &str) -> Option<Share> {
    let marker = path.find("smb-share:")?;
    let spec = &path[marker + "smb-share:".len()..];
    let spec = spec.split('/').next()?;
    let host = spec.split(',').find_map(|part| part.strip_prefix("server="))?;
    Some(Share {
        kind: "smb",
        host: Some(host.to_string()),
        port: Some(445),
    })
}

pub fn port_for(kind: &str) -> Option<u16> {
    match kind {
        "smb" => Some(445),
        "nfs" => Some(2049),
        "sftp" => Some(22),
        _ => None,
    }
}

#[cfg(windows)]
pub fn share_of(path: &Path) -> Share {
    let text = path.to_string_lossy();
    if text.starts_with("\\\\") || text.starts_with("//") {
        return Share {
            kind: "smb",
            host: host_of(&text),
            port: Some(445),
        };
    }
    let root = crate::drives::volume_root(path).unwrap_or_default();
    if crate::drives::drive_type(&root) == 4 {
        let host = crate::drives::mapped_share(&root).and_then(|s| host_of(&s));
        return Share {
            kind: "smb",
            host,
            port: Some(445),
        };
    }
    Share {
        kind: "local",
        host: None,
        port: None,
    }
}

#[cfg(unix)]
pub fn share_of(path: &Path) -> Share {
    if let Some(share) = gvfs_share(&path.to_string_lossy()) {
        return share;
    }
    let Some(mount) = crate::drives::mount_of(path, &crate::drives::mounts()) else {
        return Share {
            kind: "local",
            host: None,
            port: None,
        };
    };
    let kind = match mount.file_system.as_str() {
        "cifs" | "smb3" | "smbfs" => "smb",
        "nfs" | "nfs4" => "nfs",
        "fuse.sshfs" => "sftp",
        fs if crate::drives::is_network(fs) => "network",
        _ => "local",
    };
    let host = if kind == "local" { None } else { host_of(&mount.source) };
    Share {
        kind,
        host,
        port: port_for(kind),
    }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Un contenu pseudo-aléatoire : un NAS qui compresse ou déduplique ne doit pas fausser le débit.
fn fill(buffer: &mut [u8], seed: &mut u64) {
    for chunk in buffer.chunks_mut(8) {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        let bytes = seed.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
}

struct Meter<'a> {
    progress: ToolProgress,
    last: Instant,
    emit: &'a mut dyn FnMut(&ToolProgress),
}

impl Meter<'_> {
    fn phase(&mut self, phase: &str, total: u64) {
        self.progress.phase = phase.into();
        self.progress.bytes = 0;
        self.progress.total_bytes = total;
        (self.emit)(&self.progress);
    }

    fn advance(&mut self, bytes: u64) {
        self.progress.bytes += bytes;
        if self.last.elapsed() >= Duration::from_millis(200) {
            (self.emit)(&self.progress);
            self.last = Instant::now();
        }
    }
}

/// Lit des fichiers déjà présents, les plus gros d'abord, jusqu'à `budget` octets.
fn read_existing(root: &Path, budget: u64, control: &Control, meter: &mut Meter) -> Option<Transfer> {
    let mut candidates: Vec<(PathBuf, u64)> = walkdir::WalkDir::new(root)
        .min_depth(1)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with(TEST_DIR_PREFIX))
        .take_while(|_| !control.is_cancelled())
        .take(50_000)
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok().map(|m| (e.into_path(), m.len())))
        .filter(|(_, size)| *size >= 1 << 20)
        .take(2000)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    meter.phase("read", budget.min(candidates.iter().map(|(_, s)| s).sum()));
    let mut buffer = vec![0u8; CHUNK];
    let mut transfer = Transfer::default();
    let started = Instant::now();
    let mut window = (Instant::now(), 0u64);
    'files: for (path, _) in candidates {
        let Ok(mut file) = fs::File::open(&path) else {
            continue;
        };
        meter.progress.current = path.display().to_string();
        loop {
            if control.is_cancelled() || transfer.bytes >= budget {
                break 'files;
            }
            match file.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    transfer.bytes += n as u64;
                    meter.advance(n as u64);
                    if window.0.elapsed() >= Duration::from_millis(250) {
                        transfer.samples.push((transfer.bytes - window.1) as f64 / window.0.elapsed().as_secs_f64());
                        window = (Instant::now(), transfer.bytes);
                    }
                }
            }
        }
    }
    transfer.seconds = started.elapsed().as_secs_f64();
    transfer.speed = if transfer.seconds > 0.0 {
        transfer.bytes as f64 / transfer.seconds
    } else {
        0.0
    };
    (transfer.bytes > 0).then_some(transfer)
}

/// Les fichiers et le dossier créés par le test, retirés un par un.
struct Scratch {
    dir: PathBuf,
    files: Vec<PathBuf>,
}

impl Scratch {
    fn clean(&mut self) -> bool {
        let mut ok = true;
        for file in self.files.drain(..) {
            if file.exists() && fs::remove_file(&file).is_err() {
                ok = false;
            }
        }
        // Le dossier n'est retiré que vide : s'il contenait autre chose, il reste.
        if self.dir.exists() && fs::remove_dir(&self.dir).is_err() {
            ok = false;
        }
        ok
    }
}

fn write_test(root: &Path, options: &NetworkOptions, control: &Control, meter: &mut Meter, report: &mut NetworkReport) {
    let mut scratch = Scratch {
        dir: root.join(format!("{TEST_DIR_PREFIX}{}", crate::new_id())),
        files: Vec::new(),
    };
    if fs::create_dir(&scratch.dir).is_err() {
        report.writable = Some(false);
        return;
    }
    let big = scratch.dir.join("debit.bin");
    scratch.files.push(big.clone());
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut buffer = vec![0u8; CHUNK];
    let mut written = Md5::new();
    let mut transfer = Transfer::default();
    meter.phase("write", options.size);
    meter.progress.current = big.display().to_string();
    let started = Instant::now();
    let mut window = (Instant::now(), 0u64);
    let outcome = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&big)?;
        while transfer.bytes < options.size && !control.is_cancelled() {
            fill(&mut buffer, &mut seed);
            let n = buffer.len().min((options.size - transfer.bytes) as usize);
            file.write_all(&buffer[..n])?;
            written.update(&buffer[..n]);
            transfer.bytes += n as u64;
            meter.advance(n as u64);
            if window.0.elapsed() >= Duration::from_millis(250) {
                transfer.samples.push((transfer.bytes - window.1) as f64 / window.0.elapsed().as_secs_f64());
                window = (Instant::now(), transfer.bytes);
            }
        }
        // Le débit compte jusqu'à ce que le partage confirme que tout est écrit.
        file.sync_all()
    })();
    transfer.seconds = started.elapsed().as_secs_f64();
    transfer.speed = if transfer.seconds > 0.0 {
        transfer.bytes as f64 / transfer.seconds
    } else {
        0.0
    };
    report.writable = Some(outcome.is_ok());
    if outcome.is_ok() && !control.is_cancelled() {
        report.write = Some(transfer.clone());
        // Relecture et comparaison des empreintes : les données arrivent-elles intactes ?
        let expected = format!("{:x}", written.finalize());
        meter.phase("verify", transfer.bytes);
        let mut hasher = Md5::new();
        let mut back = Transfer::default();
        let started = Instant::now();
        let read = (|| -> std::io::Result<()> {
            let mut file = fs::File::open(&big)?;
            loop {
                if control.is_cancelled() {
                    return Ok(());
                }
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    return Ok(());
                }
                hasher.update(&buffer[..n]);
                back.bytes += n as u64;
                meter.advance(n as u64);
            }
        })();
        back.seconds = started.elapsed().as_secs_f64();
        back.speed = if back.seconds > 0.0 { back.bytes as f64 / back.seconds } else { 0.0 };
        if read.is_ok() && !control.is_cancelled() {
            report.verified = Some(format!("{:x}", hasher.finalize()) == expected && back.bytes == transfer.bytes);
            if report.read.is_none() {
                report.read = Some(back);
                report.read_from_test = true;
            }
        }
        // Petits fichiers : ce qui ralentit le plus une sauvegarde de photos ou de documents.
        if !control.is_cancelled() {
            meter.phase("small", SMALL_FILES as u64);
            let started = Instant::now();
            let mut done = 0;
            for i in 0..SMALL_FILES {
                if control.is_cancelled() {
                    break;
                }
                let path = scratch.dir.join(format!("petit-{i}.bin"));
                scratch.files.push(path.clone());
                if fs::write(&path, &buffer[..4096]).is_err() || fs::remove_file(&path).is_err() {
                    break;
                }
                done += 1;
                meter.advance(1);
            }
            if done == SMALL_FILES {
                report.small_files_per_second = Some(done as f64 / started.elapsed().as_secs_f64().max(1e-6));
            }
        }
    }
    report.cleaned = Some(scratch.clean());
}

pub fn test(path: &Path, options: &NetworkOptions, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<NetworkReport> {
    let started = Instant::now();
    let mut meter = Meter {
        progress: ToolProgress::default(),
        last: Instant::now(),
        emit,
    };
    let mut report = NetworkReport {
        path: path.display().to_string(),
        ..NetworkReport::default()
    };

    // 1. Le serveur : nom résolu, puis connexions TCP successives vers le service de partage.
    meter.phase("connect", 0);
    let share = share_of(path);
    report.kind = share.kind.into();
    report.host = share.host.clone();
    report.port = share.port;
    if let (Some(host), Some(port)) = (share.host.as_deref(), share.port) {
        let resolving = Instant::now();
        let address = (host, port).to_socket_addrs().ok().and_then(|mut a| a.next());
        report.resolve_ms = Some(ms(resolving));
        if let Some(address) = address {
            report.address = Some(address.ip().to_string());
            let mut samples = Vec::new();
            let mut failures = 0;
            for _ in 0..10 {
                if control.is_cancelled() {
                    break;
                }
                let connecting = Instant::now();
                match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
                    Ok(stream) => {
                        samples.push(ms(connecting));
                        drop(stream);
                    }
                    Err(_) => failures += 1,
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            report.reachable = !samples.is_empty();
            report.connect = Some(Latency::from(samples, failures));
        }
    }

    // 2. Le partage : accès au dossier, liste de son contenu, temps de réponse.
    meter.phase("list", 0);
    let listing = Instant::now();
    // L'ouverture se fait à part : un serveur éteint peut faire attendre le système près d'une
    // minute, et l'utilisateur doit pouvoir arrêter le test pendant ce temps.
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(fs::read_dir(&owned).map(|entries| entries.count() as u64));
    });
    let opened = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => break result,
            Err(_) if control.is_cancelled() || listing.elapsed() > Duration::from_secs(30) => {
                break Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "le partage ne répond pas"));
            }
            Err(_) => {}
        }
    };
    match opened {
        Ok(entries) => {
            report.readable = true;
            report.entries = entries;
            report.list_ms = ms(listing);
            if share.host.is_none() {
                report.reachable = true;
            }
        }
        Err(error) => {
            report.error = Some(error.to_string());
            report.elapsed = started.elapsed().as_secs_f64();
            report.cancelled = control.is_cancelled();
            if error.kind() != std::io::ErrorKind::TimedOut && !path.exists() && share.host.is_none() {
                return fail_text("err.path-inaccessible");
            }
            return Ok(report);
        }
    }
    let mut samples = Vec::new();
    let mut failures = 0;
    for _ in 0..20 {
        if control.is_cancelled() {
            break;
        }
        let asking = Instant::now();
        match fs::metadata(path) {
            Ok(_) => samples.push(ms(asking)),
            Err(_) => failures += 1,
        }
    }
    report.metadata = Latency::from(samples, failures);

    // 3. Lecture de fichiers existants, sans les modifier.
    if !control.is_cancelled() {
        report.read = read_existing(path, options.size, control, &mut meter);
    }

    // 4. Écriture, seulement si l'utilisateur l'a demandée.
    if options.write_test && !control.is_cancelled() {
        write_test(path, options, control, &mut meter, &mut report);
    }

    report.cancelled = control.is_cancelled();
    report.elapsed = started.elapsed().as_secs_f64();
    meter.progress.current.clear();
    (meter.emit)(&meter.progress);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_found_in_share_addresses() {
        assert_eq!(host_of("\\\\nas\\photos").as_deref(), Some("nas"));
        assert_eq!(host_of("//192.168.1.20/backup").as_deref(), Some("192.168.1.20"));
        assert_eq!(host_of("nas.local:/volume1/backup").as_deref(), Some("nas.local"));
        assert_eq!(host_of("moi@serveur:/home/moi").as_deref(), Some("serveur"));
        assert_eq!(host_of("/dev/sdb1"), None);
        let gvfs = gvfs_share("/run/user/1000/gvfs/smb-share:server=nas,share=photos/2024").unwrap();
        assert_eq!(gvfs.host.as_deref(), Some("nas"));
    }

    #[test]
    fn latency_statistics() {
        let latency = Latency::from(vec![1.0, 3.0, 2.0], 1);
        assert_eq!(latency.min, 1.0);
        assert_eq!(latency.max, 3.0);
        assert_eq!(latency.average, 2.0);
        assert_eq!(latency.jitter, 1.5);
        assert_eq!(latency.failures, 1);
    }

    #[test]
    fn a_local_folder_is_measured_and_left_exactly_as_it_was() {
        let lab = tempfile::tempdir().unwrap();
        fs::write(lab.path().join("existant.bin"), vec![3u8; 3 << 20]).unwrap();
        fs::write(lab.path().join("note.txt"), b"garder").unwrap();
        let before: Vec<_> = fs::read_dir(lab.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        let options = NetworkOptions {
            write_test: true,
            size: 4 << 20,
        };
        let report = test(lab.path(), &options, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.kind, "local");
        assert!(report.readable && report.reachable);
        assert_eq!(report.entries, 2);
        assert!(report.read.as_ref().is_some_and(|r| r.bytes == 3 << 20), "lecture des fichiers existants");
        assert!(!report.read_from_test);
        assert_eq!(report.writable, Some(true));
        assert_eq!(report.write.as_ref().map(|w| w.bytes), Some(4 << 20));
        assert_eq!(report.verified, Some(true));
        assert!(report.small_files_per_second.is_some());
        assert_eq!(report.cleaned, Some(true));
        let mut after: Vec<_> = fs::read_dir(lab.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        after.sort();
        let mut before = before;
        before.sort();
        assert_eq!(before, after, "aucun fichier de test ne reste");
        assert_eq!(fs::read(lab.path().join("note.txt")).unwrap(), b"garder");
    }

    #[test]
    fn without_write_test_nothing_is_created() {
        let lab = tempfile::tempdir().unwrap();
        let report = test(lab.path(), &NetworkOptions::default(), &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.writable, None);
        assert!(report.read.is_none());
        assert_eq!(fs::read_dir(lab.path()).unwrap().count(), 0);
    }
}
