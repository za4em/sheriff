use std::sync::Arc;

use futures_lite::StreamExt;
use lapin::{
    BasicProperties, Channel, Confirmation, Connection, ConnectionProperties, Consumer,
    options::{
        BasicConsumeOptions, BasicPublishOptions, BasicQosOptions, ConfirmSelectOptions,
        QueueDeclareOptions,
    },
    types::FieldTable,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

use crate::error::{AppError, queue};

mod delivery;

pub(crate) use delivery::{JobDelivery, REQUEUE_PAUSE};

const SCORE_QUEUE: &str = "sheriff.score";
const CONSUMER_TAG: &str = "sheriff-score";
const PERSISTENT_DELIVERY: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScoreJob {
    pub(crate) decision_id: i64,
}

struct Session {
    connection: Arc<Connection>,
    publisher: Channel,
}

/// Publishes score jobs and runs the one consumer that feeds workers.
#[derive(Clone)]
pub(crate) struct Queue {
    url: String,
    session: Arc<Mutex<Option<Session>>>,
}

/// Shared inbox. The lock is held only while taking the next job, not while scoring it.
#[derive(Clone)]
pub(crate) struct Jobs {
    incoming: Arc<Mutex<mpsc::Receiver<JobDelivery>>>,
}

impl Jobs {
    pub(crate) async fn recv(&self) -> Option<JobDelivery> {
        self.incoming.lock().await.recv().await
    }
}

impl Queue {
    pub(crate) async fn connect(url: &str) -> Result<Self, AppError> {
        let session = Self::open_session(url)
            .await
            .map_err(|err| queue("connecting to the broker", err))?;
        Ok(Self {
            url: url.to_owned(),
            session: Arc::new(Mutex::new(Some(session))),
        })
    }

    pub(crate) async fn publish(&self, job: &ScoreJob) -> Result<(), AppError> {
        let body = serde_json::to_vec(job).map_err(|err| queue("encoding a score job", err))?;
        for attempt in 0..2 {
            match self.publish_once(&body).await {
                Ok(()) => return Ok(()),
                Err(err) if attempt == 0 => {
                    tracing::warn!(error = %err, "score publish failed, reconnecting");
                    self.reconnect().await;
                }
                Err(err) => return Err(err),
            }
        }
        unreachable!("the second publish attempt returns")
    }

    /// One task owns the consumer and hands each delivery to a worker.
    /// Prefetch matches the worker count, so the broker does not park a backlog in this process.
    pub(crate) fn listen(self, workers: usize) -> Jobs {
        let prefetch = u16::try_from(workers).unwrap_or(u16::MAX);
        let (tx, rx) = mpsc::channel(workers.max(1));
        tokio::spawn(async move { dispatch(self, prefetch, tx).await });
        Jobs {
            incoming: Arc::new(Mutex::new(rx)),
        }
    }

    async fn publish_once(&self, body: &[u8]) -> Result<(), AppError> {
        let channel = {
            let guard = self.session.lock().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| queue("publishing a score job", "the broker is not connected"))?;
            session.publisher.clone()
        };
        let confirm = channel
            .basic_publish(
                "".into(),
                SCORE_QUEUE.into(),
                BasicPublishOptions::default(),
                body,
                BasicProperties::default().with_delivery_mode(PERSISTENT_DELIVERY),
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

    async fn reconnect(&self) {
        let mut guard = self.session.lock().await;
        match Self::open_session(&self.url).await {
            Ok(opened) => {
                let old = guard.replace(opened);
                drop(guard);
                if let Some(old) = old {
                    let _ = old.connection.close(200, "reconnect".into()).await;
                }
            }
            Err(err) => {
                tracing::error!(error = %err, "rabbitmq reconnect failed");
                *guard = None;
            }
        }
    }

    async fn connection(&self) -> Result<Arc<Connection>, AppError> {
        let guard = self.session.lock().await;
        guard
            .as_ref()
            .map(|session| Arc::clone(&session.connection))
            .ok_or_else(|| queue("connecting to the broker", "the broker is not connected"))
    }

    async fn open_consumer(&self, prefetch: u16) -> Result<Consumer, AppError> {
        let connection = match self.connection().await {
            Ok(connection) => connection,
            Err(_) => {
                self.reconnect().await;
                self.connection().await?
            }
        };
        match consume(connection.as_ref(), prefetch).await {
            Ok(consumer) => Ok(consumer),
            Err(err) => {
                self.reconnect().await;
                let connection = self.connection().await?;
                consume(connection.as_ref(), prefetch).await.or(Err(err))
            }
        }
    }

    async fn open_session(url: &str) -> Result<Session, lapin::Error> {
        let connection = Arc::new(Connection::connect(url, ConnectionProperties::default()).await?);
        let publisher = connection.create_channel().await?;
        publisher
            .confirm_select(ConfirmSelectOptions::default())
            .await?;
        declare_score_queue(&publisher).await?;
        tracing::info!("connected to rabbitmq");
        Ok(Session {
            connection,
            publisher,
        })
    }
}

async fn dispatch(queue: Queue, prefetch: u16, tx: mpsc::Sender<JobDelivery>) {
    loop {
        let mut consumer = match queue.open_consumer(prefetch).await {
            Ok(consumer) => consumer,
            Err(err) => {
                tracing::error!(error = %err, "score consumer could not connect");
                tokio::time::sleep(REQUEUE_PAUSE).await;
                continue;
            }
        };
        loop {
            match consumer.next().await {
                Some(Ok(delivery)) => match serde_json::from_slice(&delivery.data) {
                    Ok(job) => {
                        if tx.send(JobDelivery::new(job, delivery)).await.is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "score job could not be read");
                        let _ = delivery
                            .ack(lapin::options::BasicAckOptions::default())
                            .await;
                    }
                },
                Some(Err(err)) => {
                    tracing::error!(error = %err, "score consumer failed");
                    break;
                }
                None => {
                    tracing::error!("score consumer ended");
                    break;
                }
            }
        }
        tokio::time::sleep(REQUEUE_PAUSE).await;
    }
}

async fn consume(connection: &Connection, prefetch: u16) -> Result<Consumer, AppError> {
    let channel = connection
        .create_channel()
        .await
        .map_err(|err| queue("opening a score channel", err))?;
    declare_score_queue(&channel)
        .await
        .map_err(|err| queue("declaring the score queue", err))?;
    channel
        .basic_qos(prefetch, BasicQosOptions::default())
        .await
        .map_err(|err| queue("setting score prefetch", err))?;
    channel
        .basic_consume(
            SCORE_QUEUE.into(),
            CONSUMER_TAG.into(),
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await
        .map_err(|err| queue("consuming score jobs", err))
}

async fn declare_score_queue(channel: &Channel) -> Result<(), lapin::Error> {
    channel
        .queue_declare(
            SCORE_QUEUE.into(),
            QueueDeclareOptions::durable(),
            FieldTable::default(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapin::options::{BasicAckOptions, BasicGetOptions, QueuePurgeOptions};

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
    #[ignore = "needs a disposable RabbitMQ at RABBITMQ_URL"]
    async fn a_score_job_round_trips_through_the_broker() {
        let url = std::env::var("RABBITMQ_URL").unwrap();
        assert!(
            url.contains("localhost") || url.contains("127.0.0.1"),
            "refusing to purge a non-local broker queue"
        );
        let queue = Queue::connect(&url).await.unwrap();
        let guard = queue.session.lock().await;
        let channel = guard.as_ref().unwrap().publisher.clone();
        channel
            .queue_purge(SCORE_QUEUE.into(), QueuePurgeOptions::default())
            .await
            .unwrap();
        drop(guard);
        let job = job();
        queue.publish(&job).await.unwrap();
        let delivery = channel
            .basic_get(SCORE_QUEUE.into(), BasicGetOptions::default())
            .await
            .unwrap()
            .unwrap();
        let read: ScoreJob = serde_json::from_slice(&delivery.data).unwrap();
        assert_eq!(read, job);
        delivery.ack(BasicAckOptions::default()).await.unwrap();
    }
}
