use std::time::Duration;

use lapin::{
    message::Delivery,
    options::{BasicAckOptions, BasicNackOptions},
};

use super::ScoreJob;
use crate::error::{AppError, queue};

pub(crate) const REQUEUE_PAUSE: Duration = Duration::from_secs(1);

/// One broker delivery plus the job decoded from its body.
pub(crate) struct JobDelivery {
    pub(crate) job: ScoreJob,
    delivery: Delivery,
}

impl JobDelivery {
    pub(super) fn new(job: ScoreJob, delivery: Delivery) -> Self {
        Self { job, delivery }
    }

    pub(crate) async fn ack(self) -> Result<(), AppError> {
        self.delivery
            .ack(BasicAckOptions::default())
            .await
            .map(|_| ())
            .map_err(|err| queue("acknowledging a score job", err))
    }

    /// Puts the job back on the queue. The caller decides whether to wait first.
    pub(crate) async fn requeue(self) -> Result<(), AppError> {
        self.delivery
            .nack(BasicNackOptions {
                multiple: false,
                requeue: true,
            })
            .await
            .map(|_| ())
            .map_err(|err| queue("requeueing a score job", err))
    }
}
