use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes, BytesMut};
use serde::Serialize;
use tauri::State;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::error::{AppError, AppResult};
use crate::mediamtx;
use crate::network::MEDIAMTX_RTMP_PORT;
use crate::platform::{is_window_alive, is_window_minimized, restore_window_silent};
use crate::SharedState;

const INGEST_BUFFER_SIZE: usize = 64 * 1024;
const PARSER_CAPACITY: usize = 256 * 1024;
const STATS_LOG_EVERY_N_FRAMES: u64 = 60;

pub type FrameSender = Arc<watch::Sender<Option<Bytes>>>;

pub fn ingest_port_for_slot(slot: u8) -> u16 { 9000 + slot as u16 }

#[derive(Serialize, Clone, Debug)]
pub enum StreamMode {
    Mjpeg,
    Webrtc,
    WebrtcPlus,
    WebrtcVp9,
}

impl StreamMode {
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "MJPEG" => Ok(Self::Mjpeg),
            "Webrtc" => Ok(Self::Webrtc),
            "WebrtcPlus" => Ok(Self::WebrtcPlus),
            "WebrtcVp9" => Ok(Self::WebrtcVp9),
            other => Err(AppError::Msg(format!("Modalità non valida: {}", other))),
        }
    }

    pub fn is_webrtc(&self) -> bool {
        matches!(self, Self::Webrtc | Self::WebrtcPlus | Self::WebrtcVp9)
    }
}

pub struct StreamSession {
    pub title: String,
    pub mode: StreamMode,
    pub backend: CaptureBackend,
    /// Latest-frame channel for MJPEG viewers. Each frame published here
    /// replaces the previous one — slow viewers always jump to the newest
    /// frame instead of accumulating a backlog. None for WebRTC sessions.
    pub frame_tx: Option<FrameSender>,
    pub ingest_task: Option<tauri::async_runtime::JoinHandle<()>>,
    /// X11/Win32 keepalive task that watches the source HWND. None on
    /// Wayland (no HWND — FFmpeg EOF handles cleanup instead).
    pub keepalive_task: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Optional extra cleanup for platform-specific resources (e.g. PipeWire pump).
    pub on_shutdown: Option<Box<dyn FnOnce() + Send>>,
}

/// Capture backend for a stream session.
pub enum CaptureBackend {
    /// FFmpeg sidecar path (X11 / Win32). Uses Tauri CommandChild.
    Ffmpeg {
        child: Arc<std::sync::Mutex<Option<CommandChild>>>,
    },
    /// FFmpeg spawned via std::process (Wayland raw stdin path).
    #[cfg(target_os = "linux")]
    FfmpegStd {
        child: Arc<std::sync::Mutex<Option<std::process::Child>>>,
    },
}

pub fn shutdown_session(session: StreamSession) {
    if let Some(f) = session.on_shutdown { f(); }
    match session.backend {
        CaptureBackend::Ffmpeg { child } => {
            if let Some(c) = child.lock().unwrap().take() {
                let _ = c.kill();
            }
        }
        #[cfg(target_os = "linux")]
        CaptureBackend::FfmpegStd { child } => {
            if let Some(mut c) = child.lock().unwrap().take() {
                let _ = c.kill();
            }
        }
    }
    if let Some(task) = session.ingest_task { task.abort(); }
    if let Some(task) = session.keepalive_task { task.abort(); }
}

#[derive(Serialize, Clone)]
pub struct StreamInfo {
    pub slot: u8,
    pub title: String,
    pub mode: StreamMode,
}

fn any_webrtc_active(state: &SharedState) -> bool {
    state.sessions.lock().unwrap().values().any(|s| s.mode.is_webrtc())
}

/// Remove a session by slot, kill its FFmpeg + tasks, and stop MediaMTX if no
/// WebRTC streams remain. Returns true if a session was actually removed.
fn remove_and_cleanup(state: &SharedState, slot: u8) -> bool {
    let session = state.sessions.lock().unwrap().remove(&slot);
    let Some(session) = session else { return false };
    let was_webrtc = session.mode.is_webrtc();
    shutdown_session(session);
    if was_webrtc && !any_webrtc_active(state) {
        mediamtx::stop(state);
    }
    true
}

/// Drain all WebRTC sessions. Called when MediaMTX dies unexpectedly — those
/// streams can no longer reach any viewer, so kill their FFmpeg processes too.
/// Caller must NOT hold the sessions lock; may hold the mediamtx lock.
pub fn shutdown_all_webrtc(state: &SharedState) {
    let mut sessions = state.sessions.lock().unwrap();
    let webrtc_slots: Vec<u8> = sessions
        .iter()
        .filter(|(_, s)| s.mode.is_webrtc())
        .map(|(&slot, _)| slot)
        .collect();
    for slot in webrtc_slots {
        if let Some(session) = sessions.remove(&slot) {
            shutdown_session(session);
        }
    }
}

/// Drain all sessions of any kind. Called on app shutdown.
pub fn shutdown_all(state: &SharedState) {
    let drained: Vec<StreamSession> = state.sessions.lock().unwrap()
        .drain().map(|(_, s)| s).collect();
    for session in drained {
        shutdown_session(session);
    }
}

/// Incremental parser for FFmpeg's `mpjpeg` muxer output.
///
/// Each frame is emitted as `--ffmpeg\r\nContent-type: image/jpeg\r\n
/// Content-length: N\r\n\r\n<N bytes>\r\n`. We don't care about the boundary
/// or trailers — we just look for the next blank line that ends a header
/// block, parse Content-length, and emit the next N bytes as one JPEG frame.
struct MjpegParser {
    buf: BytesMut,
    state: ParserState,
}

#[derive(Clone, Copy)]
enum ParserState {
    Headers,
    Body { remaining: usize },
}

impl MjpegParser {
    fn new() -> Self {
        Self { buf: BytesMut::with_capacity(PARSER_CAPACITY), state: ParserState::Headers }
    }

    fn feed<F: FnMut(Bytes)>(&mut self, data: &[u8], mut on_frame: F) -> Result<(), &'static str> {
        self.buf.extend_from_slice(data);
        loop {
            match self.state {
                ParserState::Headers => {
                    let Some(end) = find_double_crlf(&self.buf) else { return Ok(()); };
                    let len = parse_content_length(&self.buf[..end])
                        .ok_or("missing or unparseable Content-length")?;
                    self.buf.advance(end + 4);
                    self.state = ParserState::Body { remaining: len };
                }
                ParserState::Body { remaining } => {
                    if self.buf.len() < remaining { return Ok(()); }
                    let frame = self.buf.split_to(remaining).freeze();
                    self.state = ParserState::Headers;
                    on_frame(frame);
                }
            }
        }
    }
}

/// Wrap a single JPEG frame in the multipart envelope expected by browsers
/// reading `multipart/x-mixed-replace; boundary=ffmpeg`. One allocation per
/// frame; the resulting `Bytes` is then Arc-shared across all viewers.
pub fn wrap_multipart(jpeg: Bytes) -> Bytes {
    let len_str = jpeg.len().to_string();
    let mut out = BytesMut::with_capacity(jpeg.len() + 64 + len_str.len());
    out.extend_from_slice(b"--ffmpeg\r\nContent-Type: image/jpeg\r\nContent-Length: ");
    out.extend_from_slice(len_str.as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(&jpeg);
    out.extend_from_slice(b"\r\n");
    out.freeze()
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    const NEEDLE: &[u8] = b"content-length:";
    let n = NEEDLE.len();
    let mut i = 0;
    while i + n <= headers.len() {
        if headers[i..i + n].eq_ignore_ascii_case(NEEDLE) {
            let mut j = i + n;
            while j < headers.len() && (headers[j] == b' ' || headers[j] == b'\t') { j += 1; }
            let start = j;
            while j < headers.len() && headers[j].is_ascii_digit() { j += 1; }
            if j == start { return None; }
            return std::str::from_utf8(&headers[start..j]).ok()?.parse().ok();
        }
        i += 1;
    }
    None
}

/// Per-slot timing tracker. Logs a one-line summary every N frames so jitter
/// (avg/max gap) and viewer count are visible without per-frame spam.
struct IngestStats {
    frames: u64,
    bytes: u64,
    window_start: Instant,
    last_frame: Option<Instant>,
    max_gap_ms: u64,
}

impl IngestStats {
    fn new() -> Self {
        Self {
            frames: 0,
            bytes: 0,
            window_start: Instant::now(),
            last_frame: None,
            max_gap_ms: 0,
        }
    }

    fn record(&mut self, slot: u8, viewers: usize, size: usize) {
        let now = Instant::now();
        if let Some(prev) = self.last_frame {
            let gap = now.duration_since(prev).as_millis() as u64;
            if gap > self.max_gap_ms { self.max_gap_ms = gap; }
        }
        self.last_frame = Some(now);
        self.frames += 1;
        self.bytes += size as u64;

        if self.frames >= STATS_LOG_EVERY_N_FRAMES {
            let elapsed = now.duration_since(self.window_start).as_millis().max(1) as u64;
            let avg_gap = elapsed / self.frames;
            let kbps = self.bytes * 8 / elapsed;
            tracing::debug!(
                "[ingest slot {}] frames={} elapsed={}ms avg_gap={}ms max_gap={}ms ~{}kbps viewers={}",
                slot, self.frames, elapsed, avg_gap, self.max_gap_ms, kbps, viewers
            );
            self.frames = 0;
            self.bytes = 0;
            self.window_start = now;
            self.max_gap_ms = 0;
        }
    }
}

async fn ingest_loop(slot: u8, listener: TcpListener, sender: FrameSender) {
    let mut buf = vec![0u8; INGEST_BUFFER_SIZE];
    loop {
        let (mut socket, addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[ingest slot {}] accept error: {}", slot, e);
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        // Local TCP, but disable Nagle so FFmpeg's small writes (the trailing
        // boundary) reach us promptly without a 40ms ack-delay penalty.
        let _ = socket.set_nodelay(true);
        tracing::info!("[ingest slot {}] ffmpeg connesso da {}", slot, addr);

        let mut parser = MjpegParser::new();
        let mut stats = IngestStats::new();

        loop {
            match socket.read(&mut buf).await {
                Ok(0) => {
                    tracing::info!("[ingest slot {}] ffmpeg disconnesso", slot);
                    break;
                }
                Ok(n) => {
                    let result = parser.feed(&buf[..n], |jpeg| {
                        let size = jpeg.len();
                        let viewers = sender.receiver_count();
                        // Wrap each JPEG in its multipart envelope once, here,
                        // so the HTTP handler is a straight pass-through and
                        // every viewer shares the same Arc-backed Bytes.
                        let payload = wrap_multipart(jpeg);
                        // send_replace stores the latest frame even when no
                        // viewers are subscribed, so the first viewer to
                        // connect immediately sees the freshest frame instead
                        // of waiting for the next FFmpeg packet. Plain send()
                        // would return Err and drop the frame.
                        sender.send_replace(Some(payload));
                        stats.record(slot, viewers, size);
                    });
                    if let Err(e) = result {
                        tracing::warn!("[ingest slot {}] parse error: {} - dropping connection", slot, e);
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("[ingest slot {}] read error: {}", slot, e);
                    break;
                }
            }
        }
    }
}

async fn keepalive_loop(slot: u8, hwnd: isize, state: SharedState) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    loop {
        interval.tick().await;

        if !is_window_alive(hwnd) {
            tracing::info!("[keepalive slot {}] finestra chiusa dall'utente, fermo stream", slot);
            remove_and_cleanup(&state, slot);
            break;
        }

        if is_window_minimized(hwnd) {
            tracing::debug!("[keepalive slot {}] finestra minimizzata, ripristino", slot);
            restore_window_silent(hwnd);
        }
    }
}

/// Platform-specific FFmpeg input args: which capture device, how the
/// target window is identified. On Windows that's `gdigrab` + `-i title=`;
/// on Linux X11 that's `x11grab` + `-window_id` (XComposite-tracked) with
/// the X display as the input URL. Everything else (codec, filter, muxer)
/// is shared across platforms in `build_ffmpeg_args_with_capture`.
#[cfg(windows)]
fn capture_input_args(_hwnd: isize, title: &str) -> Vec<String> {
    vec![
        "-f".into(), "gdigrab".into(),
        "-framerate".into(), "30".into(),
        "-i".into(), format!("title={}", title),
    ]
}

#[cfg(target_os = "linux")]
fn capture_input_args(hwnd: isize, _title: &str) -> Vec<String> {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    vec![
        "-f".into(), "x11grab".into(),
        "-framerate".into(), "30".into(),
        "-window_id".into(), format!("0x{:x}", hwnd as u32),
        "-i".into(), display,
    ]
}

/// Build the full FFmpeg arg list for a given mode + caller-provided
/// capture-input args. Used by both the platform-native path
/// (gdigrab/x11grab via `capture_input_args`) and the Wayland portal
/// path (libpipewire with a portal-granted node id).
fn build_ffmpeg_args_with_capture(
    mode: &StreamMode,
    capture: Vec<String>,
    output_url: &str,
) -> Vec<String> {
    let prelude: &[&str] = &[
        "-hide_banner",
        "-loglevel", "info",
        "-nostats",
        "-fflags", "nobuffer",
        "-probesize", "32",
        "-analyzeduration", "0",
        "-thread_queue_size", "512",
    ];
    let specific: &[&str] = match mode {
        StreamMode::Mjpeg => &[
            "-vf", "fps=30,mpdecimate=max=30",
            "-fps_mode", "vfr",
            "-c:v", "mjpeg",
            "-q:v", "5",
            "-flush_packets", "1",
            "-f", "mpjpeg",
        ],
        StreamMode::Webrtc => &[
            "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-tune", "zerolatency",
            "-g", "30",
            "-b:v", "2M",
            "-pix_fmt", "yuv420p",
            "-f", "flv",
        ],
        StreamMode::WebrtcPlus => &[
            "-vf", "scale=2*iw:2*ih:flags=neighbor",
            "-c:v", "libx264",
            "-preset", "fast",
            "-tune", "zerolatency",
            "-crf", "18",
            "-pix_fmt", "yuv420p",
            "-g", "30",
            "-f", "flv",
        ],
        StreamMode::WebrtcVp9 => &[
            "-vf", "scale=2*iw:2*ih:flags=neighbor",
            "-c:v", "libvpx-vp9",
            "-crf", "18",
            "-b:v", "0",
            "-pix_fmt", "yuv444p",
            "-g", "30",
            "-tune-content", "screen",
            "-f", "flv",
        ],
    };
    prelude
        .iter()
        .map(|s| (*s).to_string())
        .chain(capture.into_iter())
        .chain(specific.iter().map(|s| (*s).to_string()))
        .chain(std::iter::once(output_url.to_string()))
        .collect()
}

/// Bind the per-slot MJPEG ingest TCP listener and create the latest-frame
/// watch channel. Returns `(None, None)` for non-MJPEG modes.
async fn maybe_bind_mjpeg_listener(
    slot: u8,
    mode: &StreamMode,
) -> AppResult<(Option<FrameSender>, Option<TcpListener>)> {
    if !matches!(mode, StreamMode::Mjpeg) {
        return Ok((None, None));
    }
    let port = ingest_port_for_slot(slot);
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| AppError::Msg(format!("Bind ingest port {}: {}", port, e)))?;
    let (tx, _) = watch::channel::<Option<Bytes>>(None);
    Ok((Some(Arc::new(tx)), Some(listener)))
}

/// FFmpeg output URL: RTMP for WebRTC (via MediaMTX), local TCP for MJPEG.
fn output_url_for(slot: u8, mode: &StreamMode) -> String {
    if mode.is_webrtc() {
        format!("rtmp://127.0.0.1:{}/slot{}", MEDIAMTX_RTMP_PORT, slot)
    } else {
        format!("tcp://127.0.0.1:{}", ingest_port_for_slot(slot))
    }
}

/// Spawn the ingest + keepalive tasks (where applicable) and register the
/// session into shared state. Returns the StreamInfo describing the new
/// session. Caller is responsible for spawning the FFmpeg-process watcher
/// afterwards (the API differs between sidecar and std::process).
fn register_session(
    state: &SharedState,
    slot: u8,
    title: String,
    mode: StreamMode,
    backend: CaptureBackend,
    frame_tx: Option<FrameSender>,
    listener: Option<TcpListener>,
    keepalive_hwnd: Option<isize>,
    on_shutdown: Option<Box<dyn FnOnce() + Send>>,
) -> StreamInfo {
    let ingest_task = match (listener, frame_tx.clone()) {
        (Some(l), Some(tx)) => Some(tauri::async_runtime::spawn(async move {
            ingest_loop(slot, l, tx).await;
        })),
        _ => None,
    };

    let keepalive_task = keepalive_hwnd.map(|hwnd| {
        let state_for_ka = state.clone();
        tauri::async_runtime::spawn(async move {
            keepalive_loop(slot, hwnd, state_for_ka).await;
        })
    });

    let info = StreamInfo {
        slot,
        title: title.clone(),
        mode: mode.clone(),
    };

    state.sessions.lock().unwrap().insert(slot, StreamSession {
        title,
        mode,
        backend,
        frame_tx,
        ingest_task,
        keepalive_task,
        on_shutdown,
    });

    info
}

/// Shared launch path used by both the Win32/X11 `start_stream` and the
/// Linux Wayland `start_wayland_stream`. Caller pre-validates the slot,
/// parses the mode, and produces FFmpeg's capture-input args. Pass
/// `keepalive_hwnd = Some(hwnd)` to enable the X11/Win32 minimize/close
/// watchdog; pass `None` on Wayland (FFmpeg EOF triggers cleanup via the
/// existing event watcher below).
async fn launch_stream(
    app: &tauri::AppHandle,
    state: &SharedState,
    slot: u8,
    title: String,
    stream_mode: StreamMode,
    capture_args: Vec<String>,
    keepalive_hwnd: Option<isize>,
) -> AppResult<StreamInfo> {
    if state.sessions.lock().unwrap().contains_key(&slot) {
        return Err(AppError::SlotInUse(slot));
    }

    if stream_mode.is_webrtc() {
        mediamtx::ensure(app, state).await?;
    }

    let (frame_tx, listener) = maybe_bind_mjpeg_listener(slot, &stream_mode).await?;
    let output_url = output_url_for(slot, &stream_mode);

    let sidecar = app.shell().sidecar("ffmpeg")
        .map_err(|e| AppError::Ffmpeg(format!("sidecar non trovato: {}", e)))?;
    let args = build_ffmpeg_args_with_capture(&stream_mode, capture_args, &output_url);
    let (mut rx, child) = sidecar.args(args).spawn()
        .map_err(|e| AppError::Ffmpeg(format!("spawn fallito: {}", e)))?;
    let child = Arc::new(std::sync::Mutex::new(Some(child)));

    let info = register_session(
        state,
        slot,
        title,
        stream_mode,
        CaptureBackend::Ffmpeg { child: Arc::clone(&child) },
        frame_tx,
        listener,
        keepalive_hwnd,
        None,
    );

    // Spawn the FFmpeg event watcher. The session is already inserted so a
    // fast-fail FFmpeg termination still finds it to clean up.
    let state_for_ff = state.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(line) => {
                    tracing::info!("[ffmpeg slot {}] {}", slot, String::from_utf8_lossy(&line));
                }
                CommandEvent::Terminated(payload) => {
                    tracing::warn!("[ffmpeg slot {}] terminato: code={:?}", slot, payload.code);
                    remove_and_cleanup(&state_for_ff, slot);
                }
                _ => {}
            }
        }
    });

    Ok(info)
}

#[tauri::command]
pub async fn start_stream(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    slot: u8,
    hwnd: isize,
    window_title: String,
    mode: String,
) -> AppResult<StreamInfo> {
    if !(1..=4).contains(&slot) {
        return Err(AppError::InvalidSlot(slot));
    }
    let stream_mode = StreamMode::parse(&mode)?;

    // If the window is minimized, unminimize without stealing focus.
    if is_window_minimized(hwnd) {
        restore_window_silent(hwnd);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    let capture = capture_input_args(hwnd, &window_title);
    launch_stream(&app, state.inner(), slot, window_title, stream_mode, capture, Some(hwnd)).await
}

/// Resolve the FFmpeg binary path from the current executable's directory.
#[cfg(target_os = "linux")]
fn resolve_ffmpeg_path() -> AppResult<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or(AppError::Ffmpeg("no parent dir".into()))?;
    let path = dir.join("binaries").join("ffmpeg-x86_64-unknown-linux-gnu");
    if path.exists() {
        return Ok(path);
    }
    let output = std::process::Command::new("which")
        .arg("ffmpeg")
        .output()
        .map_err(|e| AppError::Ffmpeg(format!("which ffmpeg: {}", e)))?;
    if output.status.success() {
        let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !p.is_empty() {
            return Ok(std::path::PathBuf::from(p));
        }
    }
    Err(AppError::Ffmpeg("not found".into()))
}

/// Wayland portal capture path using native PipeWire frame reading.
/// The user must first call `wayland_select_sources` to grant access;
/// this command then reads frames from the PipeWire stream directly.
///
/// Unlike X11/Win32, the portal often returns the *full monitor resolution*
/// (e.g. 8192x4608).  FFmpeg is therefore fed rawvideo on stdin and a
/// `fps=30,scale=1280:-2:flags=fast_bilinear` pre-filter is injected so the
/// encoder never has to process more than HD-sized frames.
#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn start_wayland_stream(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    wayland: State<'_, crate::wayland_api::WaylandData>,
    slot: u8,
    source_id: u64,
    mode: String,
) -> AppResult<StreamInfo> {
    if !(1..=4).contains(&slot) {
        return Err(AppError::InvalidSlot(slot));
    }
    let stream_mode = StreamMode::parse(&mode)?;

    if state.sessions.lock().unwrap().contains_key(&slot) {
        return Err(AppError::SlotInUse(slot));
    }

    let (node_id, label) = wayland
        .lookup(source_id)
        .ok_or_else(|| AppError::Msg(format!("Wayland source {} non trovato", source_id)))?;

    let fd = wayland
        .dup_fd()?
        .ok_or(AppError::Portal(
            "FD non disponibile — eseguire wayland_select_sources prima".into(),
        ))?;

    // Probe the negotiated format on a blocking thread so we know exactly
    // what width/height/pix_fmt to tell FFmpeg.
    let format = tokio::task::spawn_blocking(move || {
        crate::pipewire_capture::probe_format(fd, node_id)
    })
    .await
    .map_err(|e| AppError::Pipewire(format!("probe join: {}", e)))?
    .map_err(AppError::Pipewire)?;

    tracing::info!(
        "[wayland slot {}] probed format: {}x{} {}",
        slot, format.width, format.height, format.pix_fmt
    );

    if stream_mode.is_webrtc() {
        mediamtx::ensure(&app, &state).await?;
    }

    let (frame_tx, listener) = maybe_bind_mjpeg_listener(slot, &stream_mode).await?;
    let output_url = output_url_for(slot, &stream_mode);

    // Pre-filter: clamp to 30 fps and scale to HD immediately so the encoder
    // never has to chew on 8K raw frames.  `fast_bilinear` is the cheapest
    // software scaler and keeps CPU usage low.
    let prefilter = "fps=30,scale=1280:-2:flags=fast_bilinear";

    let mut args: Vec<String> = vec![
        "-hide_banner".into(), "-loglevel".into(), "info".into(), "-nostats".into(),
        "-fflags".into(), "nobuffer".into(),
        "-probesize".into(), "32".into(),
        "-analyzeduration".into(), "0".into(),
        "-thread_queue_size".into(), "512".into(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), format.pix_fmt.into(),
        "-s".into(), format!("{}x{}", format.width, format.height),
        "-r".into(), "30".into(),
        "-i".into(), "pipe:0".into(),
    ];

    match stream_mode {
        StreamMode::Mjpeg => {
            args.extend([
                "-vf".into(), prefilter.into(),
                "-c:v".into(), "mjpeg".into(),
                "-q:v".into(), "5".into(),
                "-flush_packets".into(), "1".into(),
                "-f".into(), "mpjpeg".into(),
            ]);
        }
        StreamMode::Webrtc => {
            args.extend([
                "-vf".into(), format!("{},scale=trunc(iw/2)*2:trunc(ih/2)*2", prefilter),
                "-c:v".into(), "libx264".into(),
                "-preset".into(), "ultrafast".into(),
                "-tune".into(), "zerolatency".into(),
                "-g".into(), "30".into(),
                "-b:v".into(), "2M".into(),
                "-pix_fmt".into(), "yuv420p".into(),
                "-f".into(), "flv".into(),
            ]);
        }
        StreamMode::WebrtcPlus => {
            args.extend([
                "-vf".into(), format!("{},scale=2*iw:2*ih:flags=neighbor", prefilter),
                "-c:v".into(), "libx264".into(),
                "-preset".into(), "fast".into(),
                "-tune".into(), "zerolatency".into(),
                "-crf".into(), "18".into(),
                "-pix_fmt".into(), "yuv420p".into(),
                "-g".into(), "30".into(),
                "-f".into(), "flv".into(),
            ]);
        }
        StreamMode::WebrtcVp9 => {
            args.extend([
                "-vf".into(), format!("{},scale=2*iw:2*ih:flags=neighbor", prefilter),
                "-c:v".into(), "libvpx-vp9".into(),
                "-crf".into(), "18".into(),
                "-b:v".into(), "0".into(),
                "-pix_fmt".into(), "yuv444p".into(),
                "-g".into(), "30".into(),
                "-tune-content".into(), "screen".into(),
                "-f".into(), "flv".into(),
            ]);
        }
    }
    args.push(output_url);

    let ffmpeg_path = resolve_ffmpeg_path()?;
    let mut child = std::process::Command::new(&ffmpeg_path)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Ffmpeg(format!("spawn: {}", e)))?;

    // From here on, any early-return must kill the child to avoid leaking an
    // orphan FFmpeg process holding stdin/output ports.
    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            return Err(AppError::Ffmpeg("stdin not available".into()));
        }
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            return Err(AppError::Ffmpeg("stderr not available".into()));
        }
    };

    // Log FFmpeg stderr on a dedicated thread.
    let slot_for_log = slot;
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        for line in BufReader::new(stderr).lines().flatten() {
            tracing::info!("[ffmpeg slot {}] {}", slot_for_log, line);
        }
    });

    // Start the PipeWire pump synchronously *before* registering the session.
    // This eliminates the previous race where on_shutdown could fire before
    // the pump handle had been stored, leaving the pump running indefinitely.
    let fd2 = match wayland.dup_fd() {
        Ok(Some(fd)) => fd,
        Ok(None) => {
            let _ = child.kill();
            return Err(AppError::Portal("FD non disponibile per pump".into()));
        }
        Err(e) => {
            let _ = child.kill();
            return Err(e.into());
        }
    };
    let mut stdin_writer = stdin;
    let writer = Box::new(move |data: &[u8]| {
        use std::io::Write;
        stdin_writer.write_all(data).is_ok()
    }) as Box<dyn FnMut(&[u8]) -> bool + Send>;
    let pump = match crate::pipewire_capture::start_raw_pump(fd2, node_id, writer) {
        Ok(h) => h,
        Err(e) => {
            let _ = child.kill();
            return Err(AppError::Pipewire(e));
        }
    };

    let child = Arc::new(std::sync::Mutex::new(Some(child)));

    // Pump is owned by the closure; calling stop() flips the pump's atomic
    // alive flag and quits its mainloop. No Arc<Mutex<>> needed because the
    // pump is created synchronously above before the session is registered.
    let mut pump = pump;
    let on_shutdown: Option<Box<dyn FnOnce() + Send>> =
        Some(Box::new(move || pump.stop()));

    let info = register_session(
        state.inner(),
        slot,
        label,
        stream_mode,
        CaptureBackend::FfmpegStd { child: Arc::clone(&child) },
        frame_tx,
        listener,
        None,
        on_shutdown,
    );

    // Watch for FFmpeg process exit and clean up the session.
    let state_for_ff = state.inner().clone();
    let watch_slot = slot;
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            if let Some(mut c) = child.lock().unwrap().take() {
                let _ = c.wait();
            }
        }).await;
        tracing::warn!("[ffmpeg slot {}] exited", watch_slot);
        remove_and_cleanup(&state_for_ff, watch_slot);
    });

    Ok(info)
}

#[tauri::command]
pub fn stop_stream(state: State<'_, SharedState>, slot: u8) -> AppResult<()> {
    if remove_and_cleanup(state.inner(), slot) {
        Ok(())
    } else {
        Err(AppError::NoStream(slot))
    }
}

#[tauri::command]
pub fn list_streams(state: State<'_, SharedState>) -> Vec<StreamInfo> {
    state.sessions.lock().unwrap()
        .iter()
        .map(|(&slot, s)| StreamInfo {
            slot,
            title: s.title.clone(),
            mode: s.mode.clone(),
        })
        .collect()
}
