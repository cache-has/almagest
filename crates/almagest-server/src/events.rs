// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `GET /api/almagest/events` WebSocket.
//!
//! Each connected client subscribes to the shared broadcast channel and
//! receives [`ServerEvent`]s (dashboard edits, cache invalidation) plus a
//! periodic heartbeat so a dropped connection is detected. This keeps multiple
//! tabs viewing the same file consistent. Per-client subscription *filtering* is
//! a later refinement; for now every client sees every event.

use crate::state::{AppState, ServerEvent};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

/// Heartbeat cadence — keeps proxies from idling the socket and lets clients
/// notice a silent drop.
const HEARTBEAT: Duration = Duration::from_secs(30);

/// Upgrade the connection and start pumping events.
pub async fn ws_events(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.events.subscribe();
    let mut ticker = tokio::time::interval(HEARTBEAT);
    // The first tick fires immediately; skip it so we don't beat on connect.
    ticker.tick().await;

    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(event) => {
                    if send(&mut socket, &event).await.is_err() {
                        break;
                    }
                }
                // We fell behind the broadcast buffer; drop the gap and continue.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            _ = ticker.tick() => {
                if send(&mut socket, &ServerEvent::Heartbeat).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                // The client may send pings/messages; a close or error ends the loop.
                Some(Ok(_)) => continue,
                _ => break,
            },
        }
    }
}

/// Serialize and send one event as a JSON text frame.
async fn send(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).expect("ServerEvent serializes");
    socket.send(Message::Text(json.into())).await
}
