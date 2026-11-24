//! Répartition des liaisons sur les disques.
//!
//! Deux opérations ne tournent en même temps que si elles n'utilisent aucun disque commun :
//! un disque mécanique sollicité par deux copies perd plus de débit qu'il n'en gagne.

use std::collections::HashSet;
use std::sync::mpsc;

struct Done(Option<(mpsc::Sender<Vec<String>>, Vec<String>)>);

impl Drop for Done {
    fn drop(&mut self) {
        if let Some((tx, volumes)) = self.0.take() {
            let _ = tx.send(volumes);
        }
    }
}

pub struct Job<T> {
    pub volumes: Vec<String>,
    pub item: T,
}

/// Exécute `run` pour chaque élément. `skip` reçoit ceux qu'un arrêt a empêchés de démarrer.
pub fn run_jobs<T: Send>(jobs: Vec<Job<T>>, parallel: bool, stop: &(dyn Fn() -> bool + Sync), run: &(dyn Fn(T) + Sync), skip: &(dyn Fn(T) + Sync)) {
    let mut pending = jobs;
    let mut busy: HashSet<String> = HashSet::new();
    let mut running = 0usize;
    let (done_tx, done_rx) = mpsc::channel::<Vec<String>>();

    std::thread::scope(|scope| loop {
        if stop() {
            for job in pending.drain(..) {
                skip(job.item);
            }
        }
        while let Some(index) = pending
            .iter()
            .position(|job| (parallel || running == 0) && job.volumes.iter().all(|v| !busy.contains(v)))
        {
            let job = pending.remove(index);
            busy.extend(job.volumes.iter().cloned());
            running += 1;
            let tx = done_tx.clone();
            let volumes = job.volumes;
            let item = job.item;
            scope.spawn(move || {
                // Le signal de fin part même si `run` échoue brutalement : sans lui, l'attente
                // ci-dessous ne finirait jamais.
                let _done = Done(Some((tx, volumes)));
                run(item);
            });
        }
        if running == 0 && pending.is_empty() {
            break;
        }
        if let Ok(volumes) = done_rx.recv() {
            for volume in volumes {
                busy.remove(&volume);
            }
            running -= 1;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    fn job(volumes: &[&str], id: usize) -> Job<usize> {
        Job {
            volumes: volumes.iter().map(|v| v.to_string()).collect(),
            item: id,
        }
    }

    #[test]
    fn jobs_sharing_a_disk_never_overlap() {
        let active: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
        let collisions = AtomicUsize::new(0);
        let jobs = vec![job(&["D", "G"], 0), job(&["E", "G"], 1), job(&["F", "H"], 2), job(&["D", "H"], 3)];
        let volumes: Vec<Vec<String>> = jobs.iter().map(|j| j.volumes.clone()).collect();
        run_jobs(
            jobs,
            true,
            &|| false,
            &|id| {
                let mine = &volumes[id];
                {
                    let mut set = active.lock().unwrap();
                    if mine.iter().any(|v| set.contains(v)) {
                        collisions.fetch_add(1, Ordering::SeqCst);
                    }
                    set.extend(mine.iter().cloned());
                }
                std::thread::sleep(Duration::from_millis(30));
                let mut set = active.lock().unwrap();
                for v in mine {
                    set.remove(v);
                }
            },
            &|_| {},
        );
        assert_eq!(collisions.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn independent_disks_run_together() {
        let peak = AtomicUsize::new(0);
        let current = AtomicUsize::new(0);
        run_jobs(
            vec![job(&["D", "G"], 0), job(&["E", "H"], 1)],
            true,
            &|| false,
            &|_| {
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(60));
                current.fetch_sub(1, Ordering::SeqCst);
            },
            &|_| {},
        );
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stopping_skips_what_has_not_started() {
        let ran = AtomicUsize::new(0);
        let skipped = AtomicUsize::new(0);
        let stop = std::sync::atomic::AtomicBool::new(false);
        run_jobs(
            vec![job(&["D"], 0), job(&["D"], 1), job(&["D"], 2)],
            true,
            &|| stop.load(Ordering::SeqCst),
            &|_| {
                ran.fetch_add(1, Ordering::SeqCst);
                stop.store(true, Ordering::SeqCst);
            },
            &|_| {
                skipped.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(skipped.load(Ordering::SeqCst), 2);
    }
}
