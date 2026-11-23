use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::oneshot::{self, error::RecvError};

use super::SocketSender;

pub(super) enum Waiter {
    /// `/v1/score` is waiting until the row is terminal. The row is the result.
    Reply(oneshot::Sender<()>),
    /// WebSocket caller waiting for the decision itself.
    Socket(SocketSender),
}

/// Waiters for a decision id on this instance. Postgres holds the result.
///
/// The socket and the `/v1/score` call stay here. A worker on any instance
/// records the id when the row is ready.
#[derive(Clone, Default)]
pub(super) struct WaitList {
    pending: Arc<Mutex<HashMap<i64, Waiter>>>,
}

/// Removes the reply when dropped, unless the observer already took it.
pub(super) struct ScoreWait {
    list: WaitList,
    decision_id: i64,
    receiver: oneshot::Receiver<()>,
    armed: bool,
}

impl ScoreWait {
    pub(super) async fn finished(mut self) -> Result<(), RecvError> {
        let result = (&mut self.receiver).await;
        self.armed = false;
        result
    }
}

impl Drop for ScoreWait {
    fn drop(&mut self) {
        if self.armed {
            self.list.cancel_reply(self.decision_id);
        }
    }
}

impl WaitList {
    pub(super) fn reply(&self, decision_id: i64) -> ScoreWait {
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(decision_id, Waiter::Reply(sender));
        ScoreWait {
            list: self.clone(),
            decision_id,
            receiver,
            armed: true,
        }
    }

    pub(super) fn socket(&self, decision_id: i64, socket: SocketSender) {
        self.lock().insert(decision_id, Waiter::Socket(socket));
    }

    pub(super) fn take(&self, decision_id: i64) -> Option<Waiter> {
        self.lock().remove(&decision_id)
    }

    fn cancel_reply(&self, decision_id: i64) {
        let mut pending = self.lock();
        if matches!(pending.get(&decision_id), Some(Waiter::Reply(_))) {
            pending.remove(&decision_id);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<i64, Waiter>> {
        self.pending.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::core::decisions::Decision;

    #[tokio::test]
    async fn dropping_a_score_wait_removes_only_that_reply() {
        let list = WaitList::default();
        let first = list.reply(1);
        let second = list.reply(2);
        drop(first);
        assert!(list.take(1).is_none());
        assert!(matches!(list.take(2), Some(Waiter::Reply(_))));
        drop(second);
    }

    #[tokio::test]
    async fn a_finished_score_wait_leaves_the_list_alone() {
        let list = WaitList::default();
        let wait = list.reply(1);
        let Some(Waiter::Reply(sender)) = list.take(1) else {
            panic!("reply was not registered");
        };
        sender.send(()).unwrap();
        wait.finished().await.unwrap();
        assert!(list.take(1).is_none());
    }

    #[tokio::test]
    async fn a_timed_out_score_wait_removes_its_reply() {
        let list = WaitList::default();
        let wait = list.reply(7);
        let result = tokio::time::timeout(Duration::from_millis(20), wait.finished()).await;
        assert!(result.is_err());
        assert!(list.take(7).is_none());
    }

    #[test]
    fn canceling_a_reply_leaves_a_socket() {
        let list = WaitList::default();
        let (socket, _incoming) = mpsc::unbounded_channel::<Decision>();
        list.socket(1, socket);
        list.cancel_reply(1);
        assert!(matches!(list.take(1), Some(Waiter::Socket(_))));
    }
}
