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
    core::{
        Engine,
        engine::{Score, ScoreReply},
        messages::IncomingMessage,
    },
    error::AppError,
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
    ws.on_upgrade(move |socket| session(socket, engine, id))
}

async fn session(mut socket: WebSocket, engine: Engine, tenant_id: String) {
    let (reply, mut scores) = mpsc::unbounded_channel::<Score>();
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
            score = scores.recv() => {
                let Some(score) = score else {
                    break;
                };
                if !send_json(&mut socket, &score).await {
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
    reply: &ScoreReply,
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
