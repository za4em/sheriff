use lapin::{
    message::Delivery,
    options::{BasicAckOptions, BasicNackOptions},
};

use super::{REQUEUE_PAUSE, ScoreJob};
use crate::error::{AppError, queue};

pub(crate) struct Incoming {
    pub(crate) job: ScoreJob,
    delivery: Delivery,
}

impl Incoming {
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

    pub(crate) async fn nack(self) -> Result<(), AppError> {
        tokio::time::sleep(REQUEUE_PAUSE).await;
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
