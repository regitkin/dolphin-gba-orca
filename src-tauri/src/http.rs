use axum::{
    body::Body,
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Path, State as AxumState},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_stream::wrappers::WatchStream;

use crate::input::{GamepadInput, SlotKeyState};
use crate::network::{HTTP_PORT, MEDIAMTX_WEBRTC_PORT};
use crate::SharedState;

const MJPEG_VIEWER_HTML: &str = include_str!("templates/mjpeg_viewer.html");

const WEBRTC_VIEWER_HTML: &str = include_str!("templates/webrtc_viewer.html");

const GAMEPAD_CSS: &str = include_str!("templates/gamepad.css");

const GAMEPAD_HTML: &str = include_str!("templates/gamepad.html");

const GAMEPAD_JS: &str = include_str!("templates/gamepad.js");

const VIRTUAL_CSS: &str = include_str!("templates/virtual.css");

const VIRTUAL_HTML: &str = include_str!("templates/virtual.html");

const VIRTUAL_JS: &str = include_str!("templates/virtual.js");

async fn stream_handler(
    Path(slot): Path<u8>,
    AxumState(state): AxumState<SharedState>,
) -> Response {
    let receiver = {
        let sessions = state.sessions.lock().unwrap();
        match sessions.get(&slot) {
            Some(session) => match &session.frame_tx {
                Some(tx) => tx.subscribe(),
                None => return (StatusCode::NOT_FOUND, "Stream is WebRTC-only").into_response(),
            },
            None => return (StatusCode::NOT_FOUND, "No active stream").into_response(),
        }
    };

    tracing::info!("[http slot {}] viewer connected", slot);

    let stream = WatchStream::new(receiver).filter_map(|opt| async move {
        opt.map(Ok::<_, std::io::Error>)
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "multipart/x-mixed-replace;boundary=ffmpeg")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn viewer_handler(
    Path(slot): Path<u8>,
    AxumState(state): AxumState<SharedState>,
) -> Response {
    let mode = state.sessions.lock().unwrap().get(&slot).map(|s| s.mode.clone());
    let slot_str = slot.to_string();
    let rc = state.remote_controller.load(std::sync::atomic::Ordering::Relaxed);
    // Inject the gamepad fragments first so any `{slot}` they reference still
    // gets substituted by the next .replace() pass.
    let template = match mode {
        Some(m) if m.is_webrtc() => WEBRTC_VIEWER_HTML,
        _ => MJPEG_VIEWER_HTML,
    };
    let height;
    let (css, html_frag, js) = if rc == 1 {
        height = "100%";
        (GAMEPAD_CSS, GAMEPAD_HTML, GAMEPAD_JS)
    } else if rc == 2 {
        height = "66%";
        (VIRTUAL_CSS, VIRTUAL_HTML, VIRTUAL_JS)
    } else {
        height = "100%";
        ("", "", "")
    };
    let html = template
        .replace("{gp_css}", css)
        .replace("{gp_html}", html_frag)
        .replace("{gp_js}", js)
        .replace("{slot}", &slot_str)
        .replace("{webrtc_port}", &MEDIAMTX_WEBRTC_PORT.to_string())
        .replace("{height}", height);
    Html(html).into_response()
}

async fn ws_handler(Path(slot): Path<u8>, ws: WebSocketUpgrade) -> Response {
    if !(1..=4).contains(&slot) {
        return (StatusCode::NOT_FOUND, "Invalid slot").into_response();
    }
    ws.on_upgrade(move |socket| handle_ws(socket, slot))
}

async fn handle_ws(mut socket: WebSocket, slot: u8) {
    tracing::info!("[ws slot {}] gamepad client connected", slot);
    // SlotKeyState's Drop releases every still-pressed key, so a client
    // disconnecting mid-press can't leave a key stuck on the host.
    let mut state = SlotKeyState::new(slot);
    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("[ws slot {}] recv error: {}", slot, e);
                break;
            }
        };
        match msg {
            Message::Binary(data) => {
                if let Some(input) = GamepadInput::from_bytes(&data) {
                    state.apply(&input);
                } else {
                    tracing::debug!("[ws slot {}] bad binary payload (len={})", slot, data.len());
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    tracing::info!("[ws slot {}] gamepad client disconnected", slot);
}

pub async fn run_http_server(state: SharedState) {
    let router = Router::new()
        .route("/stream/:slot", get(stream_handler))
        .route("/v/:slot", get(viewer_handler))
        .route("/ws/:slot", get(ws_handler))
        .with_state(state);

    let bind = format!("0.0.0.0:{}", HTTP_PORT);
    match TcpListener::bind(&bind).await {
        Ok(listener) => {
            tracing::info!("[http] listening on {}", bind);
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("[http] error: {}", e);
            }
        }
        Err(e) => tracing::error!("[http] bind failed on {}: {}", bind, e),
    }
}