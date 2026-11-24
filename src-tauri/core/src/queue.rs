//! File d'attente des opérations, remplie uniquement par des gestes de l'utilisateur.
//!
//! Elle vit en mémoire et n'est jamais enregistrée : à la réouverture de l'application, rien
//! ne peut repartir seul.

use std::sync::Arc;

use serde::Serialize;

use crate::model::{Mode, Task};
use crate::plan::Plan;
use crate::{fail_text, new_id, Result};

#[derive(Clone)]
pub struct Queued {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: Mode,
    /// L'analyse approuvée à l'ajout : pour un miroir, la liste des suppressions acceptées.
    pub plan: Option<Arc<Plan>>,
    /// Raison pour laquelle l'opération n'a pas pu démarrer. Elle reste visible et n'est pas retentée.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedView {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: Mode,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    pub items: Vec<QueuedView>,
    pub held: bool,
}

#[derive(Default)]
pub struct Queue {
    items: Vec<Queued>,
    held: bool,
}

impl Queue {
    pub fn view(&self) -> QueueView {
        QueueView {
            items: self
                .items
                .iter()
                .map(|item| QueuedView {
                    id: item.id.clone(),
                    task_id: item.task_id.clone(),
                    task_name: item.task_name.clone(),
                    mode: item.mode,
                    error: item.error.clone(),
                })
                .collect(),
            held: self.held,
        }
    }

    pub fn add(&mut self, task: &Task, plan: Option<Arc<Plan>>) -> Result<()> {
        if self.items.iter().any(|item| item.task_id == task.id && item.error.is_none()) {
            return fail_text("err.queue-duplicate");
        }
        self.items.retain(|item| item.task_id != task.id);
        self.items.push(Queued {
            id: new_id(),
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            mode: task.mode,
            plan,
            error: None,
        });
        Ok(())
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|item| item.id != id);
        self.held &= !self.items.is_empty();
    }

    /// Avance ou recule une opération d'un rang.
    pub fn shift(&mut self, id: &str, earlier: bool) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let other = if earlier { index.checked_sub(1) } else { Some(index + 1) };
        if let Some(other) = other.filter(|&other| other < self.items.len()) {
            self.items.swap(index, other);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.held = false;
    }

    /// Une annulation arrête l'enchaînement : l'utilisateur décide de la suite.
    pub fn hold(&mut self) {
        self.held = !self.items.is_empty();
    }

    pub fn resume(&mut self) {
        self.held = false;
    }

    /// Retire la prochaine opération à démarrer, sauf si la file est en pause.
    pub fn take_next(&mut self) -> Option<Queued> {
        if self.held {
            return None;
        }
        let index = self.items.iter().position(|item| item.error.is_none())?;
        Some(self.items.remove(index))
    }

    /// Remet en tête, avec sa raison, une opération qui n'a pas pu démarrer.
    pub fn refuse(&mut self, mut item: Queued, reason: String) {
        item.error = Some(reason);
        self.items.insert(0, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            ..Task::default()
        }
    }

    fn order(queue: &Queue) -> Vec<String> {
        queue.view().items.into_iter().map(|item| item.task_id).collect()
    }

    #[test]
    fn operations_leave_in_order_and_only_once() {
        let mut queue = Queue::default();
        queue.add(&task("a"), None).unwrap();
        queue.add(&task("b"), None).unwrap();
        assert!(queue.add(&task("a"), None).is_err());
        assert_eq!(queue.take_next().unwrap().task_id, "a");
        assert_eq!(queue.take_next().unwrap().task_id, "b");
        assert!(queue.take_next().is_none());
    }

    #[test]
    fn a_hold_stops_everything_until_resumed() {
        let mut queue = Queue::default();
        queue.add(&task("a"), None).unwrap();
        queue.hold();
        assert!(queue.take_next().is_none());
        assert_eq!(order(&queue), ["a"]);
        queue.resume();
        assert_eq!(queue.take_next().unwrap().task_id, "a");
    }

    #[test]
    fn a_hold_never_outlives_an_empty_queue() {
        let mut queue = Queue::default();
        queue.hold();
        queue.add(&task("a"), None).unwrap();
        assert_eq!(queue.take_next().unwrap().task_id, "a", "annuler sans file ne retient rien");

        queue.add(&task("b"), None).unwrap();
        queue.hold();
        let id = queue.view().items[0].id.clone();
        queue.remove(&id);
        queue.add(&task("c"), None).unwrap();
        assert!(!queue.view().held);
    }

    #[test]
    fn a_refused_operation_stays_visible_and_does_not_block_the_others() {
        let mut queue = Queue::default();
        queue.add(&task("a"), None).unwrap();
        queue.add(&task("b"), None).unwrap();
        let first = queue.take_next().unwrap();
        queue.refuse(first, "disque absent".into());
        assert_eq!(queue.take_next().unwrap().task_id, "b");
        assert!(queue.take_next().is_none(), "une opération refusée n'est pas retentée seule");
        assert_eq!(queue.view().items[0].error.as_deref(), Some("disque absent"));

        queue.add(&task("a"), None).unwrap();
        assert_eq!(order(&queue), ["a"]);
        assert!(queue.view().items[0].error.is_none());
    }

    #[test]
    fn operations_can_be_reordered_and_removed() {
        let mut queue = Queue::default();
        for id in ["a", "b", "c"] {
            queue.add(&task(id), None).unwrap();
        }
        let ids: Vec<String> = queue.view().items.into_iter().map(|item| item.id).collect();
        queue.shift(&ids[2], true);
        assert_eq!(order(&queue), ["a", "c", "b"]);
        queue.shift(&ids[0], true);
        queue.shift(&ids[1], false);
        assert_eq!(order(&queue), ["a", "c", "b"]);
        queue.remove(&ids[2]);
        assert_eq!(order(&queue), ["a", "b"]);
    }
}
