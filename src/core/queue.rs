use std::sync::Arc;
use std::time::Duration;

use futures_lite::StreamExt;
use lapin::{
    BasicProperties, Channel, Confirmation, Connection, ConnectionProperties, Consumer,
    options::{
        BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, BasicQosOptions,
        ConfirmSelectOptions, QueueDeclareOptions,
    },
    types::FieldTable,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{AppError, queue};

mod incoming;

use incoming::Incoming;

const SCORE: &str = "sheriff.score";
const CONSUMER_TAG: &str = "sheriff-score";
const PREFETCH: u16 = 200;
const REQUEUE_PAUSE: Duration = Duration::from_secs(1);
const PERSISTENT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScoreJob {
    pub(crate) decision_id: i64,
}

struct Session {
    connection: Connection,
    publisher: Channel,
}

#[derive(Clone)]
pub(crate) struct Queue {
    url: String,
    session: Arc<Mutex<Session>>,
    consumer: Arc<Mutex<Option<Consumer>>>,
}

impl Queue {
    pub(crate) async fn connect(url: &str) -> Result<Self, lapin::Error> {
        let session = Self::open(url).await?;
        Ok(Self {
            url: url.to_owned(),
            session: Arc::new(Mutex::new(session)),
            consumer: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) async fn publish(&self, job: &ScoreJob) -> Result<(), AppError> {
        let body = serde_json::to_vec(job).map_err(|err| queue("encoding a score job", err))?;
        let channel = self.session.lock().await.publisher.clone();
        let confirm = channel
            .basic_publish(
                "".into(),
                SCORE.into(),
                BasicPublishOptions::default(),
                &body,
                BasicProperties::default().with_delivery_mode(PERSISTENT),
            )
            .await
            .map_err(|err| queue("publishing a score job", err))?;
        match confirm
            .await
            .map_err(|err| queue("publishing a score job", err))?
        {
            Confirmation::Ack(_) | Confirmation::NotRequested => Ok(()),
            Confirmation::Nack(_) => Err(queue(
                "publishing a score job",
                "the broker rejected the job",
            )),
        }
    }

    /// Call this before accepting traffic.
    pub(crate) async fn ready(&self) -> Result<(), AppError> {
        let mut consumer = self.consumer.lock().await;
        if consumer.is_none() {
            *consumer = Some(self.open_consumer().await?);
        }
        Ok(())
    }

    /// Returns the next valid job. Reconnects when the consumer stops.
    pub(crate) async fn recv(&self) -> Result<Incoming, AppError> {
        loop {
            let delivery = {
                let mut consumer = self.consumer.lock().await;
                if consumer.is_none() {
                    *consumer = Some(self.open_consumer().await?);
                }
                consumer.as_mut().unwrap().next().await
            };
            match delivery {
                Some(Ok(delivery)) => match serde_json::from_slice(&delivery.data) {
                    Ok(job) => return Ok(Incoming::new(job, delivery)),
                    Err(err) => {
                        tracing::error!(error = %err, "score job could not be read");
                        let _ = delivery.ack(BasicAckOptions::default()).await;
                    }
                },
                Some(Err(err)) => {
                    tracing::error!(error = %err, "score consumer failed");
                    self.lost().await;
                }
                None => {
                    tracing::error!("score consumer ended");
                    self.lost().await;
                }
            }
        }
    }

    async fn open_consumer(&self) -> Result<Consumer, AppError> {
        let channel = self.channel().await?;
        channel
            .basic_qos(PREFETCH, BasicQosOptions::default())
            .await
            .map_err(|err| queue("setting score prefetch", err))?;
        channel
            .basic_consume(
                SCORE.into(),
                CONSUMER_TAG.into(),
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|err| queue("consuming score jobs", err))
    }

    async fn channel(&self) -> Result<Channel, AppError> {
        let session = self.session.lock().await;
        session
            .connection
            .create_channel()
            .await
            .map_err(|err| queue("opening a channel", err))
    }

    async fn lost(&self) {
        let mut consumer = self.consumer.lock().await;
        if consumer.is_none() {
            return;
        }
        consumer.take();
        if let Err(err) = self.reconnect().await {
            tracing::error!(error = %err, "rabbitmq reconnect failed");
            tokio::time::sleep(REQUEUE_PAUSE).await;
        }
    }

    async fn reconnect(&self) -> Result<(), AppError> {
        let opened = Self::open(&self.url)
            .await
            .map_err(|err| queue("reconnecting", err))?;
        let old = {
            let mut session = self.session.lock().await;
            std::mem::replace(&mut *session, opened)
        };
        let _ = old.connection.close(200, "reconnect".into()).await;
        Ok(())
    }

    async fn open(url: &str) -> Result<Session, lapin::Error> {
        let connection = Connection::connect(url, ConnectionProperties::default()).await?;
        let publisher = connection.create_channel().await?;
        publisher
            .confirm_select(ConfirmSelectOptions::default())
            .await?;
        publisher
            .queue_declare(
                SCORE.into(),
                QueueDeclareOptions::durable(),
                FieldTable::default(),
            )
            .await?;
        tracing::info!("connected to rabbitmq");
        Ok(Session {
            connection,
            publisher,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapin::options::{BasicGetOptions, QueuePurgeOptions};

    fn job() -> ScoreJob {
        ScoreJob { decision_id: 7 }
    }

    #[test]
    fn a_score_job_round_trips_through_json() {
        let job = job();
        let bytes = serde_json::to_vec(&job).unwrap();
        let read: ScoreJob = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(read, job);
    }

    #[tokio::test]
    #[ignore = "needs RabbitMQ at RABBITMQ_URL"]
    async fn a_score_job_round_trips_through_the_broker() {
        let url = std::env::var("RABBITMQ_URL").unwrap();
        let queue = Queue::connect(&url).await.unwrap();
        let channel = queue.session.lock().await.publisher.clone();
        channel
            .queue_purge(SCORE.into(), QueuePurgeOptions::default())
            .await
            .unwrap();
        let job = job();
        queue.publish(&job).await.unwrap();
        let delivery = channel
            .basic_get(SCORE.into(), BasicGetOptions::default())
            .await
            .unwrap()
            .unwrap();
        let read: ScoreJob = serde_json::from_slice(&delivery.data).unwrap();
        assert_eq!(read, job);
        delivery.ack(BasicAckOptions::default()).await.unwrap();
    }
}
