//! Outils de vérification et de rangement : comparer deux dossiers, trouver les doublons,
//! cartographier l'espace occupé, tester la lecture d'un disque, restaurer depuis les archives.
//!
//! Tous lisent sans rien modifier. Les deux seules écritures possibles sont demandées
//! explicitement par l'utilisateur : la restauration vers un dossier qu'il choisit, sans jamais
//! remplacer un fichier existant, et la mise à la corbeille de doublons, récupérable et refusée
//! si la copie conservée n'est pas strictement identique.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::engine::Control;
use crate::filters::{is_skipped_dir, ARCHIVE_DIR, LEGACY_ARCHIVE_DIR};
use crate::{fail_text, Result};

const MAX_LISTED: usize = 5000;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgress {
    /// « list », « hash » ou « read ».
    pub phase: String,
    pub files: u64,
    pub total_files: u64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub current: String,
}

struct Ticker<'a> {
    state: ToolProgress,
    last: Instant,
    emit: &'a mut dyn FnMut(&ToolProgress),
}

impl<'a> Ticker<'a> {
    fn new(emit: &'a mut dyn FnMut(&ToolProgress)) -> Self {
        Self {
            state: ToolProgress::default(),
            last: Instant::now() - Duration::from_secs(1),
            emit,
        }
    }

    fn tick(&mut self) {
        if self.last.elapsed() >= Duration::from_millis(200) {
            (self.emit)(&self.state);
            self.last = Instant::now();
        }
    }

    fn flush(&mut self) {
        (self.emit)(&self.state);
    }
}

#[derive(Debug, Clone)]
struct FileInfo {
    path: PathBuf,
    rel: String,
    size: u64,
    modified: i64,
}

fn modified_secs(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Tous les fichiers sous `root`, sans suivre les liens ni entrer dans les dossiers système.
fn list_files(root: &Path, control: &Control, ticker: &mut Ticker) -> (Vec<FileInfo>, Vec<(String, String)>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && is_skipped_dir(&e.file_name().to_string_lossy())));
    ticker.state.phase = "list".into();
    for entry in walker {
        if control.is_cancelled() {
            break;
        }
        match entry {
            Ok(entry) if entry.file_type().is_file() => match entry.metadata() {
                Ok(metadata) => {
                    ticker.state.total_files += 1;
                    ticker.state.total_bytes += metadata.len();
                    files.push(FileInfo {
                        rel: rel_of(root, entry.path()),
                        path: entry.into_path(),
                        size: metadata.len(),
                        modified: modified_secs(&metadata),
                    });
                    ticker.tick();
                }
                Err(error) => errors.push((rel_of(root, entry.path()), error.to_string())),
            },
            Ok(_) => {}
            Err(error) => errors.push((error.path().map(|p| rel_of(root, p)).unwrap_or_default(), error.to_string())),
        }
    }
    (files, errors)
}

fn wait_if_paused(control: &Control) {
    while control.is_paused() && !control.is_cancelled() {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Empreinte MD5, éventuellement limitée au début du fichier. `None` si l'utilisateur a arrêté.
fn digest(path: &Path, limit: Option<u64>, control: &Control, read: &mut dyn FnMut(u64)) -> std::io::Result<Option<String>> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        wait_if_paused(control);
        if control.is_cancelled() {
            return Ok(None);
        }
        let want = limit.map(|l| (l - total).min(buffer.len() as u64) as usize).unwrap_or(buffer.len());
        if want == 0 {
            break;
        }
        let n = file.read(&mut buffer[..want])?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buffer[..n]);
        read(n as u64);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

/// Empreintes de plusieurs fichiers, lus par `workers` fils à la fois. Le résultat suit l'ordre
/// de `files` ; un fichier illisible, ou non lu parce que l'utilisateur a arrêté, donne None.
/// Une lecture limitée (début de fichier) compte un fichier lu ; une lecture entière compte ses octets.
fn digests(files: &[FileInfo], limit: Option<u64>, workers: usize, control: &Control, ticker: &mut Ticker) -> Vec<Option<String>> {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let results: Mutex<Vec<Option<String>>> = Mutex::new(vec![None; files.len()]);
    let current = Mutex::new(String::new());
    let (base_files, base_bytes) = (ticker.state.files, ticker.state.bytes);
    let whole = limit.is_none();
    let publish = |ticker: &mut Ticker| {
        if whole {
            ticker.state.files = base_files + done.load(Ordering::Relaxed) as u64;
            ticker.state.bytes = base_bytes + bytes.load(Ordering::Relaxed);
        }
        ticker.state.current = current.lock().unwrap_or_else(|e| e.into_inner()).clone();
    };
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, files.len().max(1)) {
            scope.spawn(|| loop {
                if control.is_cancelled() {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(index) else { break };
                file.rel.clone_into(&mut current.lock().unwrap_or_else(|e| e.into_inner()));
                let mut count = |n: u64| {
                    bytes.fetch_add(n, Ordering::Relaxed);
                };
                if let Ok(Some(sum)) = digest(&file.path, limit, control, &mut count) {
                    results.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(sum);
                }
                done.fetch_add(1, Ordering::Relaxed);
            });
        }
        while done.load(Ordering::Relaxed) < files.len() && !control.is_cancelled() {
            std::thread::sleep(Duration::from_millis(50));
            publish(ticker);
            ticker.tick();
        }
    });
    publish(ticker);
    results.into_inner().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- Comparer deux dossiers

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub rel: String,
    pub size: u64,
    pub modified: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub rel: String,
    pub left: Side,
    pub right: Side,
    /// « size », « date » ou « content ».
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareReport {
    pub left: String,
    pub right: String,
    pub content: bool,
    pub same: u64,
    pub only_left_count: u64,
    pub only_right_count: u64,
    pub different_count: u64,
    pub only_left: Vec<Side>,
    pub only_right: Vec<Side>,
    pub different: Vec<Difference>,
    pub errors: Vec<(String, String)>,
    pub cancelled: bool,
    pub elapsed: f64,
}

fn side(file: &FileInfo) -> Side {
    Side {
        rel: file.rel.clone(),
        size: file.size,
        modified: file.modified,
    }
}

/// Compare deux arborescences par taille et date, et si demandé par contenu. Rien n'est copié.
pub fn compare(left: &Path, right: &Path, content: bool, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<CompareReport> {
    if !left.is_dir() || !right.is_dir() {
        return fail_text("err.path-inaccessible");
    }
    let started = Instant::now();
    let mut ticker = Ticker::new(emit);
    let (a, mut errors) = list_files(left, control, &mut ticker);
    let (b, more) = list_files(right, control, &mut ticker);
    errors.extend(more);
    let mut report = CompareReport {
        left: left.display().to_string(),
        right: right.display().to_string(),
        content,
        ..CompareReport::default()
    };
    // Windows ignore la casse : deux noms qui ne diffèrent que par elle désignent le même fichier.
    let key = |rel: &str| if cfg!(windows) { rel.to_lowercase() } else { rel.to_string() };
    let theirs: HashMap<String, &FileInfo> = b.iter().map(|f| (key(&f.rel), f)).collect();
    let ours: HashMap<String, &FileInfo> = a.iter().map(|f| (key(&f.rel), f)).collect();
    ticker.state.phase = "hash".into();
    ticker.state.files = 0;
    ticker.state.bytes = 0;
    // La progression porte sur ce qui sera vraiment lu : les deux côtés des paires de même taille.
    ticker.state.total_files = a.len() as u64;
    ticker.state.total_bytes = if content {
        a.iter()
            .filter_map(|f| theirs.get(&key(&f.rel)).filter(|o| o.size == f.size).map(|_| f.size * 2))
            .sum()
    } else {
        0
    };
    // Deux disques différents se lisent en même temps ; un même disque, l'un après l'autre.
    let disks = crate::volumes::all();
    let parallel = crate::volumes::volume_key(left, &disks) != crate::volumes::volume_key(right, &disks);
    let read = std::sync::atomic::AtomicU64::new(0);
    for file in &a {
        if control.is_cancelled() {
            break;
        }
        ticker.state.files += 1;
        ticker.state.current = file.rel.clone();
        ticker.tick();
        let Some(other) = theirs.get(&key(&file.rel)) else {
            report.only_left_count += 1;
            if report.only_left.len() < MAX_LISTED {
                report.only_left.push(side(file));
            }
            continue;
        };
        let reason = if file.size != other.size {
            Some("size")
        } else if content {
            let mut count = |n: u64| {
                read.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
            };
            let sums = if parallel {
                std::thread::scope(|scope| {
                    let theirs = scope.spawn(|| {
                        let mut count = |n: u64| {
                            read.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                        };
                        digest(&other.path, None, control, &mut count)
                    });
                    let ours = digest(&file.path, None, control, &mut count);
                    (ours, theirs.join().unwrap_or_else(|_| Err(std::io::Error::other("lecture interrompue"))))
                })
            } else {
                match digest(&file.path, None, control, &mut count) {
                    Ok(Some(ours)) => (Ok(Some(ours)), digest(&other.path, None, control, &mut count)),
                    // Le côté gauche illisible ou arrêté : inutile de lire le droit.
                    failed => (failed, Ok(Some(String::new()))),
                }
            };
            ticker.state.bytes = read.load(std::sync::atomic::Ordering::Relaxed);
            match sums {
                (Ok(Some(x)), Ok(Some(y))) => (x != y).then_some("content"),
                (Ok(None), _) | (_, Ok(None)) => break,
                (Err(e), _) | (_, Err(e)) => {
                    errors.push((file.rel.clone(), e.to_string()));
                    continue;
                }
            }
        } else if (file.modified - other.modified).abs() > 1 {
            Some("date")
        } else {
            None
        };
        match reason {
            Some(reason) => {
                report.different_count += 1;
                if report.different.len() < MAX_LISTED {
                    report.different.push(Difference {
                        rel: file.rel.clone(),
                        left: side(file),
                        right: side(other),
                        reason: reason.into(),
                    });
                }
            }
            None => report.same += 1,
        }
    }
    for file in &b {
        if !ours.contains_key(&key(&file.rel)) {
            report.only_right_count += 1;
            if report.only_right.len() < MAX_LISTED {
                report.only_right.push(side(file));
            }
        }
    }
    errors.truncate(MAX_LISTED);
    report.errors = errors;
    report.cancelled = control.is_cancelled();
    report.elapsed = started.elapsed().as_secs_f64();
    ticker.state.current.clear();
    ticker.flush();
    Ok(report)
}

// ---------------------------------------------------------------- Doublons

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub size: u64,
    pub md5: String,
    pub files: Vec<String>,
    /// Date de modification de chaque fichier lors de l'analyse, dans l'ordre de `files`.
    #[serde(default)]
    pub modified: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateReport {
    pub roots: Vec<String>,
    pub scanned: u64,
    pub groups: Vec<DuplicateGroup>,
    /// Place récupérable en ne gardant qu'un exemplaire de chaque groupe.
    pub wasted: u64,
    pub cancelled: bool,
    pub elapsed: f64,
}

/// Cherche les fichiers au contenu identique : même taille, puis même début, puis même empreinte.
pub fn duplicates(roots: &[PathBuf], min_size: u64, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<DuplicateReport> {
    let started = Instant::now();
    let mut ticker = Ticker::new(emit);
    let mut files = Vec::new();
    for root in roots {
        if !root.is_dir() {
            return fail_text("err.path-inaccessible");
        }
        let (mut found, _) = list_files(root, control, &mut ticker);
        for file in &mut found {
            file.rel = file.path.display().to_string();
        }
        files.extend(found);
    }
    let mut report = DuplicateReport {
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        scanned: files.len() as u64,
        ..DuplicateReport::default()
    };
    // Un même fichier atteint par deux dossiers choisis l'un dans l'autre ne compte qu'une fois.
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.dedup_by(|a, b| a.rel == b.rel);

    let mut by_size: BTreeMap<u64, Vec<FileInfo>> = BTreeMap::new();
    for file in files.into_iter().filter(|f| f.size >= min_size.max(1)) {
        by_size.entry(file.size).or_default().push(file);
    }
    let candidates: Vec<FileInfo> = by_size.into_values().filter(|group| group.len() > 1).flatten().collect();
    ticker.state.phase = "hash".into();
    ticker.state.total_files = candidates.len() as u64;
    ticker.state.total_bytes = candidates.iter().map(|f| f.size).sum();
    ticker.state.files = 0;
    ticker.state.bytes = 0;
    // Sur un partage réseau chaque ouverture attend le serveur : plusieurs lectures simultanées
    // masquent cette attente. Un disque local, souvent mécanique, est lu un fichier à la fois.
    let network = roots.iter().any(|root| crate::drives::is_network_path(root));
    let (head_workers, full_workers) = if network { (8, 4) } else { (2, 1) };

    // Le début du fichier écarte vite la plupart des faux candidats.
    let heads = digests(&candidates, Some(64 * 1024), head_workers, control, &mut ticker);
    let mut by_head: HashMap<(u64, String), Vec<FileInfo>> = HashMap::new();
    let mut settled = (0u64, 0u64);
    for (file, head) in candidates.into_iter().zip(heads) {
        match head {
            Some(head) => by_head.entry((file.size, head)).or_default().push(file),
            None => settled = (settled.0 + 1, settled.1 + file.size),
        }
    }
    let mut full = Vec::new();
    for group in by_head.into_values() {
        if group.len() > 1 {
            full.extend(group);
        } else {
            settled = (settled.0 + group.len() as u64, settled.1 + group.iter().map(|f| f.size).sum::<u64>());
        }
    }
    ticker.state.files = settled.0;
    ticker.state.bytes = settled.1;
    full.sort_by(|a, b| a.rel.cmp(&b.rel));

    let sums = if control.is_cancelled() { vec![None; full.len()] } else { digests(&full, None, full_workers, control, &mut ticker) };
    let mut by_sum: HashMap<(u64, String), Vec<FileInfo>> = HashMap::new();
    for (file, sum) in full.into_iter().zip(sums) {
        if let Some(sum) = sum {
            by_sum.entry((file.size, sum)).or_default().push(file);
        }
    }
    for ((size, md5), same) in by_sum.into_iter().filter(|(_, g)| g.len() > 1) {
        report.wasted += size * (same.len() as u64 - 1);
        report.groups.push(DuplicateGroup {
            size,
            md5,
            modified: same.iter().map(|f| f.modified).collect(),
            files: same.into_iter().map(|f| f.rel).collect(),
        });
    }
    report.groups.sort_by_key(|g| std::cmp::Reverse(g.size * g.files.len() as u64));
    report.groups.truncate(MAX_LISTED);
    report.cancelled = control.is_cancelled();
    report.elapsed = started.elapsed().as_secs_f64();
    ticker.state.current.clear();
    ticker.flush();
    Ok(report)
}

/// Met à la corbeille les copies choisies, après avoir vérifié qu'elles ont toujours le contenu
/// trouvé par l'analyse, comme l'exemplaire conservé, qui lui n'est jamais touché.
///
/// Un fichier resté à la même taille et à la même date depuis l'analyse n'est pas relu : relire
/// des gigaoctets sur un NAS avant chaque suppression figeait l'outil. Un fichier modifié depuis
/// est relu en entier et refusé si son contenu a changé.
pub fn trash_duplicates(group: &DuplicateGroup, keep: &str, remove: &[String], owner: Option<isize>) -> Result<crate::recycle::Recycled> {
    let control = Control::default();
    let unchanged = |path: &str| -> bool {
        let Ok(meta) = fs::metadata(path) else { return false };
        if !meta.is_file() || meta.len() != group.size {
            return false;
        }
        let seen = group.files.iter().position(|f| f == path).and_then(|i| group.modified.get(i));
        if seen == Some(&modified_secs(&meta)) {
            return true;
        }
        let mut ignore = |_: u64| {};
        matches!(digest(Path::new(path), None, &control, &mut ignore), Ok(Some(sum)) if sum == group.md5)
    };
    if remove.is_empty() || remove.iter().any(|p| p == keep || !group.files.contains(p)) || !group.files.iter().any(|f| f == keep) {
        return fail_text("err.duplicate-changed");
    }
    if !unchanged(keep) || !remove.iter().all(|p| unchanged(p)) {
        return fail_text("err.duplicate-changed");
    }
    let paths: Vec<PathBuf> = remove.iter().map(PathBuf::from).collect();
    crate::recycle::recycle(&paths, owner)
}

// ---------------------------------------------------------------- Carte de l'espace

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceNode {
    pub name: String,
    pub size: u64,
    pub files: u64,
    /// Vrai pour un fichier, ou pour le regroupement des petits éléments.
    pub leaf: bool,
    /// Vrai pour le regroupement des petits éléments : il n'existe pas sur le disque.
    pub grouped: bool,
    pub children: Vec<SpaceNode>,
}

const SPACE_DEPTH: usize = 7;
const SPACE_CHILDREN: usize = 40;

fn prune(node: &mut SpaceNode, depth: usize) {
    node.children.sort_by_key(|n| std::cmp::Reverse(n.size));
    if node.children.len() > SPACE_CHILDREN {
        let rest: Vec<SpaceNode> = node.children.split_off(SPACE_CHILDREN);
        node.children.push(SpaceNode {
            name: format!("+{}", rest.len()),
            size: rest.iter().map(|n| n.size).sum(),
            files: rest.iter().map(|n| n.files).sum(),
            leaf: true,
            grouped: true,
            children: Vec::new(),
        });
    }
    for child in &mut node.children {
        if depth >= SPACE_DEPTH {
            child.children.clear();
        } else {
            prune(child, depth + 1);
        }
    }
}

/// Taille de chaque dossier, des plus gros aux plus petits, pour voir où va la place.
pub fn space(root: &Path, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<SpaceNode> {
    if !root.is_dir() {
        return fail_text("err.path-inaccessible");
    }
    let mut ticker = Ticker::new(emit);
    let (files, _) = list_files(root, control, &mut ticker);
    // Chaque nœud indexe ses enfants par nom : retrouver un dossier ne parcourt plus tous ses
    // frères, ce qui rendait la construction quadratique sur un dossier de 100 000 photos.
    #[derive(Default)]
    struct Building {
        size: u64,
        files: u64,
        children: Vec<(String, bool, Building)>,
        index: HashMap<(String, bool), usize>,
    }
    fn finish(name: String, leaf: bool, node: Building) -> SpaceNode {
        SpaceNode {
            name,
            size: node.size,
            files: node.files,
            leaf,
            grouped: false,
            children: node.children.into_iter().map(|(name, leaf, child)| finish(name, leaf, child)).collect(),
        }
    }
    let mut building = Building::default();
    for (count, file) in files.iter().enumerate() {
        if count % 4096 == 0 && control.is_cancelled() {
            break;
        }
        let parts: Vec<&str> = file.rel.split('/').collect();
        let mut node = &mut building;
        node.size += file.size;
        node.files += 1;
        for (i, part) in parts.iter().enumerate() {
            let leaf = i == parts.len() - 1;
            let key = (part.to_string(), leaf);
            let index = match node.index.get(&key) {
                Some(&index) => index,
                None => {
                    node.children.push((key.0.clone(), leaf, Building::default()));
                    node.index.insert(key, node.children.len() - 1);
                    node.children.len() - 1
                }
            };
            node = &mut node.children[index].2;
            node.size += file.size;
            node.files += 1;
        }
    }
    let mut tree = finish(root.display().to_string(), false, building);
    prune(&mut tree, 1);
    ticker.flush();
    Ok(tree)
}

// ---------------------------------------------------------------- Test de lecture

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowFile {
    pub rel: String,
    pub size: u64,
    /// Octets par seconde.
    pub speed: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReport {
    pub root: String,
    pub files: u64,
    pub bytes: u64,
    pub elapsed: f64,
    pub average_speed: f64,
    /// Débit mesuré au fil de la lecture, pour tracer la courbe.
    pub samples: Vec<f64>,
    pub slow: Vec<SlowFile>,
    pub unreadable: Vec<(String, String)>,
    pub cancelled: bool,
}

const SLOW_MIN_SIZE: u64 = 8 * 1024 * 1024;

/// Lit chaque fichier en entier pour vérifier que le disque rend bien ses données, et signale
/// les fichiers illisibles et ceux lus anormalement lentement (zone fatiguée du disque).
pub fn read_test(root: &Path, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<ReadReport> {
    if !root.is_dir() {
        return fail_text("err.path-inaccessible");
    }
    let started = Instant::now();
    let mut ticker = Ticker::new(emit);
    let (files, errors) = list_files(root, control, &mut ticker);
    let mut report = ReadReport {
        root: root.display().to_string(),
        unreadable: errors,
        ..ReadReport::default()
    };
    ticker.state.phase = "read".into();
    ticker.state.files = 0;
    ticker.state.bytes = 0;
    let mut speeds: Vec<(usize, f64)> = Vec::new();
    let mut window = (Instant::now(), 0u64);
    for (index, file) in files.iter().enumerate() {
        if control.is_cancelled() {
            break;
        }
        ticker.state.current = file.rel.clone();
        let begin = Instant::now();
        let mut count = |n: u64| {
            ticker.state.bytes += n;
            window.1 += n;
            if window.0.elapsed() >= Duration::from_millis(500) {
                report.samples.push(window.1 as f64 / window.0.elapsed().as_secs_f64());
                window = (Instant::now(), 0);
            }
            ticker.tick();
        };
        match digest(&file.path, None, control, &mut count) {
            Ok(Some(_)) => {
                let seconds = begin.elapsed().as_secs_f64().max(1e-6);
                if file.size >= SLOW_MIN_SIZE {
                    speeds.push((index, file.size as f64 / seconds));
                }
            }
            Ok(None) => break,
            Err(error) => report.unreadable.push((file.rel.clone(), error.to_string())),
        }
        ticker.state.files += 1;
    }
    report.files = ticker.state.files;
    report.bytes = ticker.state.bytes;
    report.elapsed = started.elapsed().as_secs_f64();
    report.average_speed = if report.elapsed > 0.0 { report.bytes as f64 / report.elapsed } else { 0.0 };
    // Lent : moins du quart du débit médian des gros fichiers du même disque.
    if speeds.len() >= 4 {
        let mut sorted: Vec<f64> = speeds.iter().map(|(_, s)| *s).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];
        report.slow = speeds
            .iter()
            .filter(|(_, speed)| *speed < median / 4.0)
            .map(|(i, speed)| SlowFile {
                rel: files[*i].rel.clone(),
                size: files[*i].size,
                speed: *speed,
            })
            .take(MAX_LISTED)
            .collect();
    }
    report.unreadable.truncate(MAX_LISTED);
    report.cancelled = control.is_cancelled();
    ticker.state.current.clear();
    ticker.flush();
    Ok(report)
}

// ---------------------------------------------------------------- Archives

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveVersion {
    /// Dossier d'archive (« _Archives Chud Copy » ou l'ancien nom).
    pub archive: String,
    pub stamp: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedFile {
    pub rel: String,
    pub size: u64,
    pub modified: i64,
}

fn archive_roots(target: &Path) -> Vec<(String, PathBuf)> {
    [ARCHIVE_DIR, LEGACY_ARCHIVE_DIR]
        .iter()
        .map(|name| (name.to_string(), target.join(name)))
        .filter(|(_, path)| path.is_dir())
        .collect()
}

/// Les versions archivées d'une destination, les plus récentes d'abord.
pub fn archive_versions(target: &Path) -> Result<Vec<ArchiveVersion>> {
    let mut out = Vec::new();
    for (archive, root) in archive_roots(target) {
        for entry in fs::read_dir(&root)?.flatten().filter(|e| e.path().is_dir()) {
            let (mut files, mut bytes) = (0, 0);
            for file in walkdir::WalkDir::new(entry.path()).into_iter().flatten().filter(|e| e.file_type().is_file()) {
                files += 1;
                bytes += file.metadata().map(|m| m.len()).unwrap_or(0);
            }
            out.push(ArchiveVersion {
                archive: archive.clone(),
                stamp: entry.file_name().to_string_lossy().into_owned(),
                files,
                bytes,
            });
        }
    }
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    Ok(out)
}

fn version_dir(target: &Path, archive: &str, stamp: &str) -> Result<PathBuf> {
    let valid = |s: &str| !s.is_empty() && !s.contains(['/', '\\']) && s != "." && s != "..";
    if ![ARCHIVE_DIR, LEGACY_ARCHIVE_DIR].contains(&archive) || !valid(stamp) {
        return fail_text("err.bad-id");
    }
    Ok(target.join(archive).join(stamp))
}

pub fn archive_files(target: &Path, archive: &str, stamp: &str) -> Result<Vec<ArchivedFile>> {
    let root = version_dir(target, archive, stamp)?;
    let mut out: Vec<ArchivedFile> = walkdir::WalkDir::new(&root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let metadata = e.metadata().ok()?;
            Some(ArchivedFile {
                rel: rel_of(&root, e.path()),
                size: metadata.len(),
                modified: modified_secs(&metadata),
            })
        })
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// Copie des fichiers archivés vers un dossier choisi, en gardant leurs sous-dossiers.
/// Un fichier déjà présent n'est jamais remplacé : la copie restaurée prend un autre nom.
pub fn restore(target: &Path, archive: &str, stamp: &str, rels: &[String], destination: &Path) -> Result<Vec<String>> {
    let root = version_dir(target, archive, stamp)?;
    let mut written = Vec::new();
    for rel in rels {
        if rel.split(['/', '\\']).any(|part| part == "..") {
            return fail_text("err.bad-id");
        }
        let source = root.join(rel);
        if !source.is_file() {
            continue;
        }
        let mut path = destination.join(rel);
        if path.exists() {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let extension = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let mut n = 1;
            loop {
                let suffix = if n == 1 { " (restauré)".to_string() } else { format!(" (restauré {n})") };
                let candidate = path.with_file_name(format!("{stem}{suffix}{extension}"));
                if !candidate.exists() {
                    path = candidate;
                    break;
                }
                n += 1;
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &path)?;
        if let Ok(metadata) = fs::metadata(&source) {
            if let (Ok(modified), Ok(file)) = (metadata.modified(), fs::File::options().write(true).open(&path)) {
                let _ = file.set_modified(modified);
            }
        }
        written.push(path.display().to_string());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
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

    #[test]
    fn two_folders_are_compared_without_touching_them() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write(&a, "pareil.txt", b"x");
        write(&b, "pareil.txt", b"x");
        write(&a, "seul-a.txt", b"a");
        write(&b, "sous/seul-b.txt", b"b");
        write(&a, "taille.txt", b"court");
        write(&b, "taille.txt", b"plus long");
        write(&a, "contenu.txt", b"AAAA");
        write(&b, "contenu.txt", b"BBBB");
        // Même taille, même date, contenu différent : seul le contrôle du contenu le voit.
        let stamp = fs::metadata(a.join("contenu.txt")).unwrap().modified().unwrap();
        fs::File::options()
            .write(true)
            .open(b.join("contenu.txt"))
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let before = (snapshot(&a), snapshot(&b));

        let quick = compare(&a, &b, false, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!((quick.same, quick.only_left_count, quick.only_right_count, quick.different_count), (2, 1, 1, 1));
        assert_eq!(quick.different[0].reason, "size");

        let deep = compare(&a, &b, true, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(deep.different_count, 2);
        assert!(deep.different.iter().any(|d| d.rel == "contenu.txt" && d.reason == "content"));
        assert_eq!((snapshot(&a), snapshot(&b)), before);
    }

    #[test]
    fn duplicates_are_found_by_content_not_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![42u8; 200_000];
        write(dir.path(), "photos/vacances.jpg", &big);
        write(dir.path(), "copie/IMG_0001.jpg", &big);
        write(dir.path(), "copie/encore.jpg", &big);
        let mut almost = big.clone();
        almost[150_000] = 0;
        write(dir.path(), "autre/presque.jpg", &almost);
        write(dir.path(), "vide1.txt", b"");
        write(dir.path(), "vide2.txt", b"");

        let report = duplicates(&[dir.path().to_path_buf()], 1, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].files.len(), 3);
        assert_eq!(report.wasted, 400_000);
        assert!(!report.groups[0].files.iter().any(|f| f.contains("presque")));

        let twice = duplicates(&[dir.path().to_path_buf(), dir.path().join("copie")], 1, &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(twice.groups[0].files.len(), 3, "un fichier vu par deux chemins ne compte qu'une fois");
    }

    #[test]
    fn a_duplicate_that_changed_since_the_scan_is_never_trashed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "garde.bin", b"identique");
        write(dir.path(), "autre.bin", b"identique");
        let report = duplicates(&[dir.path().to_path_buf()], 1, &Control::default(), &mut |_| {}).unwrap();
        let group = report.groups[0].clone();
        assert_eq!(group.modified.len(), 2);
        let keep = group.files.iter().find(|f| f.ends_with("garde.bin")).unwrap().clone();
        let other = group.files.iter().find(|f| f.ends_with("autre.bin")).unwrap().clone();

        assert!(trash_duplicates(&group, &keep, &[keep.clone()], None).is_err(), "l'exemplaire conservé ne part jamais");
        assert!(trash_duplicates(&group, &keep, &[], None).is_err());
        assert!(trash_duplicates(&group, &keep, &["ailleurs.bin".into()], None).is_err(), "seuls les fichiers du groupe");

        // Même taille, autre contenu, autre date : relu puis refusé.
        fs::write(&other, b"different").unwrap();
        let file = fs::File::options().write(true).open(&other).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(120)).unwrap();
        drop(file);
        assert!(trash_duplicates(&group, &keep, &[other.clone()], None).is_err());
        assert!(Path::new(&other).exists() && Path::new(&keep).exists());
    }

    #[test]
    fn the_space_map_adds_up_and_orders_by_size() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "films/a.mkv", &vec![0u8; 5000]);
        write(dir.path(), "films/b.mkv", &vec![0u8; 3000]);
        write(dir.path(), "docs/c.txt", &[0u8; 100]);
        let tree = space(dir.path(), &Control::default(), &mut |_| {}).unwrap();
        assert_eq!((tree.size, tree.files), (8100, 3));
        assert_eq!(tree.children[0].name, "films");
        assert_eq!(tree.children[0].size, 8000);
        assert_eq!(tree.children[0].children[0].name, "a.mkv");
    }

    #[test]
    fn the_read_test_reads_everything_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.bin", &vec![1u8; 300_000]);
        write(dir.path(), "sous/b.bin", &vec![2u8; 100_000]);
        let before = snapshot(dir.path());
        let report = read_test(dir.path(), &Control::default(), &mut |_| {}).unwrap();
        assert_eq!((report.files, report.bytes), (2, 400_000));
        assert!(report.unreadable.is_empty());
        assert_eq!(snapshot(dir.path()), before);
    }

    #[test]
    fn archived_versions_restore_beside_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dst");
        write(&target, "_Archives Chud Copy/2026-11-01 10h00 00/docs/rapport.txt", b"ancienne version");
        write(&target, "_Archives Chud Copy/2026-11-02 10h00 00/photo.jpg", b"photo");
        write(&target, ".chudcopy-archive/1700000000000/vieux.txt", b"legacy");
        let versions = archive_versions(&target).unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].stamp, "2026-11-02 10h00 00");

        let files = archive_files(&target, ARCHIVE_DIR, "2026-11-01 10h00 00").unwrap();
        assert_eq!(files[0].rel, "docs/rapport.txt");

        let out = dir.path().join("restauration");
        write(&out, "docs/rapport.txt", b"version actuelle");
        let written = restore(&target, ARCHIVE_DIR, "2026-11-01 10h00 00", &["docs/rapport.txt".into()], &out).unwrap();
        assert_eq!(fs::read(out.join("docs/rapport.txt")).unwrap(), b"version actuelle", "rien n'est remplacé");
        assert_eq!(fs::read(out.join("docs/rapport (restauré).txt")).unwrap(), b"ancienne version");
        assert_eq!(written.len(), 1);
        assert!(restore(&target, ARCHIVE_DIR, "..", &["x".into()], &out).is_err());
        assert!(restore(&target, ARCHIVE_DIR, "2026-11-01 10h00 00", &["../../evasion".into()], &out).is_err());
    }
}
