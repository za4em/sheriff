use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use super::SocketSender;

pub(super) enum Notice {
    Call(oneshot::Sender<()>),
    Socket(SocketSender),
}

/// Who to tell when a decision id is finished. The row in Postgres is the result.
#[derive(Clone, Default)]
pub(super) struct WaitList {
    pending: Arc<Mutex<HashMap<i64, Notice>>>,
}

impl WaitList {
    pub(super) fn call(&self, decision_id: i64) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(decision_id, Notice::Call(sender));
        receiver
    }

    pub(super) fn socket(&self, decision_id: i64, socket: SocketSender) {
        self.lock().insert(decision_id, Notice::Socket(socket));
    }

    pub(super) fn take(&self, decision_id: i64) -> Option<Notice> {
        self.lock().remove(&decision_id)
    }

    pub(super) fn take_many(&self, decision_ids: &[i64]) -> Option<SocketSender> {
        let mut pending = self.lock();
        let mut socket = None;
        for id in decision_ids {
            if let Some(Notice::Socket(sender)) = pending.remove(id) {
                if socket.is_none() {
                    socket = Some(sender);
                }
            }
        }
        socket
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Notice>> {
        self.pending.lock().unwrap_or_else(|err| err.into_inner())
    }
}
