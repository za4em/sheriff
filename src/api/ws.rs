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

use super::tenant::Tenant;
use crate::{
    app::App,
    core::{Engine, messages::IncomingMessage},
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
    loop {
        let Some(frame) = socket.recv().await else {
            break;
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(err) => {
                tracing::debug!(error = %err, "websocket closed");
                break;
            }
        };
        if !handle_frame(&mut socket, &engine, &tenant_id, frame).await {
            break;
        }
    }
}

async fn handle_frame(
    socket: &mut WebSocket,
    engine: &Engine,
    tenant_id: &str,
    frame: Message,
) -> bool {
    match frame {
        Message::Text(text) => match serde_json::from_str::<Vec<IncomingMessage>>(text.as_str()) {
            Ok(messages) => match engine.submit_sync(tenant_id, messages).await {
                Ok(decisions) => send_json(socket, &decisions).await,
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
