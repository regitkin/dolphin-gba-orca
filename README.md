# GBA Orca

Stream Dolphin's GBA windows to phones and tablets on your local network.

When you play GameCube games that use GBA-link features, Dolphin opens separate GBA windows on your PC. GBA Orca picks up those windows and streams each one to a browser on any device connected to the same Wi-Fi or LAN. You can send inputs back to the host PC with a USB controller or with the virtual joypad.

## How it works

```
[Dolphin] ──GBA windows──▶ [GBA Orca] ──HTTP/WebRTC──▶ [Phones / tablets]
```

GBA Orca finds the GBA windows automatically, captures each one with FFmpeg, and serves them as either MJPEG over HTTP or WebRTC. Players just open a URL in their browser.
## Requirements

- Windows 10/11 **or Linux** (X11 or Wayland)
- Dolphin running a game with GBA windows
- PC and phones on the same Wi-Fi/LAN

## How to use it

1. Download the installer from [Releases](https://github.com/regitkin/dolphin-gba-orca/releases).
2. Start Dolphin and a game with controllers set to **GBA (Integrated)**. Dolphin will open the GBA windows.
3. Open GBA Orca — it lists every GBA window it sees.
4. Click **Start stream** on each one you want to share.
5. Send each player the stream URL shown in the app (something like `http://192.168.1.42:8080/v/1`).
6. On the phone, the round button at the bottom-right rotates the video 90° for landscape play.

You can select the streaming mode:

- **MJPEG** — Best quality, but high latency  
- **WebRTC** — Poor quality  
- **WebRTC++** — Surprisingly good [Recommended]
- **WebRTC VP9** — Second best quality, but CPU intensive and not widely supported

The app rescans every 3 seconds, so closing or restarting Dolphin mid-session is fine — the list updates on its own.

### Controller over Stream

 
This feature allows players to use a physical controller connected to their phone or tablet to control Dolphin running on the PC. It works with **all streaming modes** (MJPEG and WebRTC).

> **Note:** For best compatibility, use **Chrome** (latest version). **iOS** may not work correctly — to be tested.

Before clicking **Start stream**, enable the **Controller over Stream** checkbox. 

**On the viewer (phone / tablet):**
- The page auto-detects the gamepad as soon as you press any button.
- Once connected, the notification shows **"Connected"**.
- If the controller disconnects, the notification reappears automatically.
- Press **×** on the notification to permanently hide it (refresh the page to restore it).

**On the PC (Dolphin setup):**
GBA Orca converts the controller input received from the player’s device into local input events on the PC, so you must bind each slot to the correct keys in Dolphin.

1. In Dolphin, go to **Controllers → GBA (Integrated)** and pick the slot you are streaming.
2. Set the device to **Keyboard**. IMPORTANT!!
3. Map each GBA button to a keyboard key by pressing the desired key on your controller.


> **Note:** Controller over Stream currently requires **Windows**. The WebSocket endpoint is available on Linux but key injection is not yet implemented.


## Build from source

```bash
git clone https://github.com/regitkin/dolphin-gba-orca.git
cd dolphin-gba-orca
npm install
```

Install Rust with [rustup](https://rustup.rs/), then:

```bash
npm run tauri dev      # development
npm run tauri build    # production installer in src-tauri/target/release/bundle/
```

## Stack

Tauri 2 + Rust backend, Svelte frontend. FFmpeg (`gdigrab` on Windows, `x11grab` on X11, PipeWire on Wayland) for capture, MJPEG over HTTP for the stream, axum + tokio for the server. Window enumeration uses the `windows` crate on Windows and `x11rb` on X11. LAN interface discovery uses `local-ip-address`.

The axum server proxies each FFmpeg process so multiple viewers can watch the same stream — FFmpeg's built-in HTTP server can't do that. If an FFmpeg process dies (window closed, fullscreen, etc.) the session is cleaned up automatically.

## Limitations

- **Linux Wayland:** because of Wayland limitations: you must manually select the GBA windows in order. Auto-detect works fully on Windows and X11.
- **Unencrypted.** Stream is plain HTTP on the LAN — meant for home use.

## Roadmap

- Custom APP for Android and IOS with PIN-based routing (4-digit code instead of full URL)

## Contributing

Issues and PRs welcome. Project is early-stage and the internals are still moving.
