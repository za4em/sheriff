use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use serde::Serialize;
use tokio::sync::mpsc;

use super::tenant::Tenant;
use crate::{
    app::App,
    core::{Engine, decisions::Decision, engine::SocketSender, messages::IncomingMessage},
    error::AppError,
    metrics::Metrics,
};

pub(crate) fn routes() -> Router<App> {
    Router::new().route("/v1/stream", get(stream))
}

async fn stream(
    Tenant { id }: Tenant,
    State(app): State<App>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    tracing::info!(tenant = %id, "websocket connected");
    let engine = app.engine.clone();
    let metrics = app.metrics.clone();
    ws.on_upgrade(move |socket| session(socket, engine, metrics, id))
}

async fn session(mut socket: WebSocket, engine: Engine, metrics: Metrics, tenant_id: String) {
    let _connection = OpenConnection::open(metrics);
    let (reply, mut decisions) = mpsc::unbounded_channel::<Decision>();
    loop {
        tokio::select! {
            frame = socket.recv() => {
                let Some(frame) = frame else {
                    break;
                };
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(err) => {
                        tracing::debug!(error = %err, "websocket closed");
                        break;
                    }
                };
                if !handle_frame(&mut socket, &engine, &tenant_id, &reply, frame).await {
                    break;
                }
            }
            decision = decisions.recv() => {
                let Some(decision) = decision else {
                    break;
                };
                if !send_json(&mut socket, &decision).await {
                    break;
                }
            }
        }
    }
}

async fn handle_frame(
    socket: &mut WebSocket,
    engine: &Engine,
    tenant_id: &str,
    reply: &SocketSender,
    frame: Message,
) -> bool {
    match frame {
        Message::Text(text) => match serde_json::from_str::<Vec<IncomingMessage>>(text.as_str()) {
            Ok(messages) => match engine
                .submit(tenant_id, messages, Some(reply.clone()))
                .await
            {
                Ok(()) => true,
                Err(err) => send_error(socket, &err).await,
            },
            Err(_) => send_error(socket, &AppError::InvalidMessage).await,
        },
        Message::Binary(_) => send_error(socket, &AppError::InvalidMessage).await,
        Message::Close(_) => false,
        Message::Ping(_) | Message::Pong(_) => true,
    }
}

struct OpenConnection(Metrics);

impl OpenConnection {
    fn open(metrics: Metrics) -> Self {
        metrics.open_websocket();
        Self(metrics)
    }
}

impl Drop for OpenConnection {
    fn drop(&mut self) {
        self.0.close_websocket();
    }
}

#[derive(Serialize)]
struct SocketError<'a> {
    error: &'a str,
}

async fn send_error(socket: &mut WebSocket, err: &AppError) -> bool {
    let message = err.to_string();
    send_json(socket, &SocketError { error: &message }).await
}

async fn send_json(socket: &mut WebSocket, body: &impl Serialize) -> bool {
    socket
        .send(Message::Text(encode(body).into()))
        .await
        .is_ok()
}

fn encode(body: &impl Serialize) -> String {
    serde_json::to_string(body)
        .unwrap_or_else(|_| r#"{"error":"The decision could not be encoded."}"#.to_owned())
}
