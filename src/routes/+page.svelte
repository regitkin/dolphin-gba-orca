<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import QRCode from "qrcode";

  // ---- i18n ----
  const translations = {
    it: {
      refresh: "Aggiorna",
      scanning: "Scansione...",
      gbaOnly: "Solo finestre GBA",
      autoScan: "Auto-scan: 3s",
      server: "Server",
      controller: "Controller",
      controllerUsb: "USB",
      controllerNone: "None",
      controllerVirtual: "Virtual",
      clickToCopy: "Click per copiare",
      slot: "Slot",
      windowTitle: "Titolo finestra",
      size: "Dim.",
      status: "Stato",
      startStream: "Avvia stream",
      stop: "Stop",
      noWindows: "Nessuna finestra trovata",
      windowsLabel: "finestre",
      activeStreams: "stream attivi",
      interfaces: "interfacce di rete",
      language: "Lingua",
      mode: "Modalità",
      mjpeg: "MJPEG",
      webrtc: "WebRTC",
      webrtcPlus: "WebRTC++",
      webrtcVp9: "WebRTC VP9",
      waylandTitle: "Modalità Wayland",
      selectGbaWindows: "Seleziona finestre GBA",
      noWaylandSourcesYet: "Nessuna sorgente selezionata. Clicca sopra per aprire il portal.",
      waylandHintBefore: "Su Wayland l'enumerazione automatica delle finestre non è permessa. ",
      waylandHintAction: "Clicca il pulsante",
      waylandHintAfter: " per scegliere le finestre GBA in ordine tramite il portal di sistema.",
      assignSlot: "Assegna",
      sourceLabel: "Sorgente",
      waylandAlert: "WAYLAND rilevato",
      waylandSelectOrder: "Seleziona nell'ordine:",
      qr: "Codici QR",
      showQr: "Mostra QR",
      hideQr: "Nascondi QR",
      scanQr: "Scansiona il codice QR per connetterti", 
    },
    en: {
      refresh: "Refresh",
      scanning: "Scanning...",
      gbaOnly: "GBA windows only",
      autoScan: "Auto-scan: 3s",
      server: "Server",
      controller: "Controller",
      controllerUsb: "USB",
      controllerNone: "None",
      controllerVirtual: "Virtual",
      clickToCopy: "Click to copy",
      slot: "Slot",
      windowTitle: "Window title",
      size: "Size",
      status: "Status",
      startStream: "Start stream",
      stop: "Stop",
      noWindows: "No windows found",
      windowsLabel: "windows",
      activeStreams: "active streams",
      interfaces: "network interfaces",
      language: "Language",
      mode: "Mode",
      mjpeg: "MJPEG",
      webrtc: "WebRTC",
      webrtcPlus: "WebRTC++",
      webrtcVp9: "WebRTC VP9",
      waylandTitle: "Wayland mode",
      waylandHintBefore: "Wayland forbids automatic window enumeration. ",
      waylandHintAction: "Click the button",
      waylandHintAfter: " to pick the GBA windows in order through the system portal dialog.",
      selectGbaWindows: "Select GBA windows",
      noWaylandSourcesYet: "No source selected yet. Click the button above to open the portal.",
      assignSlot: "Assign",
      sourceLabel: "Source",
      waylandAlert: "WAYLAND detected",
      waylandSelectOrder: "Select in order:",
      qr: "QR Codes",
      showQr: "Show QR",
      hideQr: "Hide QR",
      scanQr: "Scan the QR code to connect",
    },
    es:{
      refresh: "Actualizar",
      scanning: "Escaneando...",
      gbaOnly: "Solo ventanas GBA",
      autoScan: "Auto-escaneo: 3s",
      server: "Servidor",
      controller: "Controlador",
      controllerUsb: "USB",
      controllerNone: "Ninguno",
      controllerVirtual: "Virtual",
      clickToCopy: "Haga clic para copiar",
      slot: "Slot",
      windowTitle: "Título de la ventana",
      size: "Tamaño",
      status: "Estado",
      startStream: "Iniciar stream",
      stop: "Detener",
      noWindows: "No se encontraron ventanas",
      windowsLabel: "ventanas",
      activeStreams: "streams activos",
      interfaces: "interfaces de red",
      language: "Idioma",
      mode: "Modo",
      mjpeg: "MJPEG",
      webrtc: "WebRTC",
      webrtcPlus: "WebRTC++",
      webrtcVp9: "WebRTC VP9",
      waylandTitle: "Modo Wayland",
      waylandHintBefore: "Wayland prohibe la enumeración automática de ventanas. ",
      waylandHintAction: "Haga clic en el botón",
      waylandHintAfter: " para seleccionar las ventanas GBA en orden a través del diálogo del portal del sistema.",
      selectGbaWindows: "Seleccionar ventanas GBA",
      noWaylandSourcesYet: "Aún no se ha seleccionado ninguna fuente. Haga clic en el botón de arriba para abrir el portal.",
      assignSlot: "Asignar",
      sourceLabel: "Fuente",
      waylandAlert: "WAYLAND detectado",
      waylandSelectOrder: "Seleccionar en orden:",
      qr: "Codigo QR",
      showQr: "Mostrar QR",
      hideQr: "Ocultar QR",
      scanQr: "Escanee el código QR para conectarse",
    }
  };

  // Settings helpers
  const STORAGE_PREFIX = "gba-orca-";

  /** @param {string} key @param {*} fallback */
  function loadSetting(key, fallback) {
    try {
      const raw = localStorage.getItem(STORAGE_PREFIX + key);
      if (raw === null) return fallback;
      return JSON.parse(raw);
    } catch (e) {
      return fallback;
    }
  }

  /** @param {string} key @param {*} value */
  function saveSetting(key, value) {
    try {
      localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify(value));
    } catch (e) {
      console.error("saveSetting:", key, e);
    }
  }

  function detectLang() {
    const saved = loadSetting("lang", null);
    if (saved && translations[saved]) return saved;
    const sys = (navigator.language || navigator.userLanguage || "en").toLowerCase();
    return sys.startsWith("it") ? "it" : "en";
  }

  let lang = detectLang();
  $: t = translations[lang];
  $: saveSetting("lang", lang);

  // Application state
  let windows = [];
  let streams = {};
  let server = { interfaces: [], port: 8080, webrtc_port: 8889 };

  let selectedIp = loadSetting("selectedIp", "");
  let gbaOnly = loadSetting("gbaOnly", true);
  let streamMode = loadSetting("streamMode", "WebrtcPlus");
  let controller = loadSetting("controller", 0);

  let loading = false;
  let error = "";
  let autoScanInterval = null;

  //Implements QR code for each stream, so the user can scan it with their phone and open the stream in a browser.
  let showQr = false;
  let qrCodes = {};

  let settingsLoaded = false;

  $: if (settingsLoaded) saveSetting("selectedIp", selectedIp);
  $: if (settingsLoaded) saveSetting("gbaOnly", gbaOnly);
  $: if (settingsLoaded) saveSetting("streamMode", streamMode);
  $: if (settingsLoaded) saveSetting("controller", controller);

  // Wayland-specific state. On Wayland we cannot enumerate windows: the user
  // must pick them via xdg-desktop-portal, then assign each captured PipeWire
  // source to a slot.
  let isWayland = false;
  let waylandSources = [];
  let waylandBusy = false;

  $: serverUrl = `http://${selectedIp || "..."}:${server.port}`;

  /** @param {number} slot @param {string} mode */
  function streamViewUrl(slot, mode) {
    return `${serverUrl}/v/${slot}`;
  }

  /*Function to generate QR codes for each stream URL.
  This will create a QR code for each of the 4 slots, which can be scanned by a mobile device to open the stream in a browser.*/
  async function generateQrCodes() {
    if (!selectedIp) return;
    const codes = {};
    for (let slot = 1; slot <= 4; slot++) {
        const url = `${serverUrl}/v/${slot}`;
        try {
          codes[slot] = await QRCode.toDataURL(url, {
            width: 220,
            margin: 2,
          });
        }catch(e) {
          console.error(`Error generating QR for slot ${slot}:`, e);
      }
    }
    qrCodes = codes;
  }
  // Toggle the visibility of QR codes. If they are not generated yet, generate them first.
  async function toggleQr() {
    if(!showQr) {
      await generateQrCodes();
    }
    showQr = !showQr;
  }
  // If the selected IP changes, regenerate the QR codes to reflect the new server URL.
  async function handleIpChange() {
    if (showQr){
        await generateQrCodes();
    }
  }

  function modeLabel(mode) {
    if (mode === "WebrtcPlus") return "WebRTC++";
    if (mode === "WebrtcVp9") return "WebRTC VP9";
    if (mode === "Webrtc") return "WebRTC";
    return "MJPEG";
  }

  async function loadServerInfo() {
    try {
      server = await invoke("get_server_info");
      const knownIps = server.interfaces.map((iface) => iface.ip);
      if (server.interfaces.length > 0 && (!selectedIp || !knownIps.includes(selectedIp))) {
        selectedIp = server.interfaces[0].ip;
      }

    } catch (e) {
      console.error("server info:", e);
    }
  }

  async function doScan(silent = false) {
    if (!silent) loading = true;
    if (!silent) error = "";
    try {
      const result = gbaOnly
        ? await invoke("list_gba_windows")
        : await invoke("list_windows");
      windows = result;
      await refreshStreams();
    } catch (e) {
      if (!silent) error = String(e);
    } finally {
      if (!silent) loading = false;
    }
  }

  async function refreshStreams() {
    try {
      const list = await invoke("list_streams");
      const map = {};
      for (const s of list) map[s.slot] = s;
      streams = map;
    } catch (e) {
      console.error(e);
    }
  }

  async function startStream(w) {
      if (!w.gba_slot) return;
      error = "";
      try {
        await invoke("start_stream", {
          slot: w.gba_slot,
          hwnd: w.hwnd,
          windowTitle: w.title,
          mode: streamMode,
        });
        await refreshStreams();
      } catch (e) {
        error = String(e);
      }
    }

  async function stopStream(slot) {
    error = "";
    try {
      await invoke("stop_stream", { slot });
      await refreshStreams();
    } catch (e) {
      error = String(e);
    }
  }

  function copyToClipboard(text) {
    navigator.clipboard?.writeText(text);
  }

  async function refreshWaylandSources() {
    try {
      waylandSources = await invoke("wayland_list_sources");
    } catch (e) {
      console.error("wayland_list_sources:", e);
    }
  }

  async function selectController() {
    try {
      await invoke("set_remote_controller", { controller });
    } catch (e) {
      console.error("set_remote_controller:", e);
    }
  }

  async function selectWaylandSources() {
    error = "";
    waylandBusy = true;
    try {
      waylandSources = await invoke("wayland_select_sources");
    } catch (e) {
      error = String(e);
    } finally {
      waylandBusy = false;
    }
  }

  async function assignWaylandSlot(source, raw) {
    const parsed = parseInt(raw, 10);
    const slot = Number.isFinite(parsed) && parsed >= 1 && parsed <= 4 ? parsed : null;
    error = "";
    try {
      waylandSources = await invoke("wayland_assign_slot", {
        sourceId: source.id,
        slot,
      });
    } catch (e) {
      error = String(e);
    }
  }

  async function startWaylandStream(source) {
    if (!source.gba_slot) return;
    error = "";
    try {
      await invoke("start_wayland_stream", {
        slot: source.gba_slot,
        sourceId: source.id,
        mode: streamMode,
      });
      await refreshStreams();
    } catch (e) {
      error = String(e);
    }
  }

  onMount(async () => {
    await loadServerInfo();

    // Load saved controller preference
    try {
      const backendController = await invoke("get_remote_controller");
      if (controller !== backendController) {
        await invoke("set_remote_controller", { controller });
      }
    } catch (e) {
      // leave the persisted/default controller value as-is
    }

    try {
      isWayland = await invoke("is_wayland");
    } catch (e) {
      isWayland = false;
    }

    // From here on, changes to persisted settings should be saved.
    settingsLoaded = true;

    if (isWayland) {
      await refreshWaylandSources();
      await refreshStreams();
      // Stream list still polls so dead FFmpeg processes free their slot.
      autoScanInterval = setInterval(refreshStreams, 3000);
    } else {
      doScan(false);
      autoScanInterval = setInterval(() => doScan(true), 3000);
    }
  });

  onDestroy(() => {
    if (autoScanInterval) clearInterval(autoScanInterval);
  });
</script>

<div class="app">

  {#if isWayland}
    <div class="wayland-banner">
      <span class="wayland-alert">{t.waylandAlert}</span>
      <span class="wayland-text"> — {t.waylandSelectOrder} </span>
      <span class="wayland-slot">GBA1</span><span class="wayland-text">, </span>
      <span class="wayland-slot">GBA2</span><span class="wayland-text">, </span>
      <span class="wayland-slot">GBA3</span><span class="wayland-text">, </span>
      <span class="wayland-slot">GBA4</span>
    </div>
  {/if}

  <div class="server-row">
    <label>{t.server}:</label>
    {#if server.interfaces.length > 1}
      <select bind:value={selectedIp} on:change={handleIpChange}>
      {#each server.interfaces as iface}
          <option value={iface.ip}>
              {iface.ip} — {iface.name}
          </option>
      {/each}
    </select>
    {/if}
    <code on:click={() => copyToClipboard(serverUrl)} title={t.clickToCopy}>
      {serverUrl}
    </code>
    <button
        class="qr-toggle"
        on:click={toggleQr}
        title={showQr ? t.hideQr : t.showQr}
    >
        {showQr ? t.hideQr : t.showQr}
    </button>
  </div>


{#if showQr}
    <div class="qr-section">
        <div class="qr-section-header">
            <strong>{t.qr}</strong>
            <span>{t.scanQr}</span>
        </div>

        <div class="qr-grid">
            {#each [1, 2, 3, 4] as slot}
                <div class="qr-item">
                    <div class="qr-slot">
                        {t.slot} {slot}
                    </div>

                    {#if qrCodes[slot]}
                        <img
                            src={qrCodes[slot]}
                            alt={`${t.qr} ${t.slot} ${slot}`}
                        />

                        <code>
                            {serverUrl}/v/{slot}
                        </code>
                    {:else}
                        <div class="qr-loading">
                            ...
                        </div>
                    {/if}
                </div>
            {/each}
        </div>
    </div>
{/if}

  <div class="toolbar">
    {#if isWayland}
      <button class="primary-btn" on:click={selectWaylandSources} disabled={waylandBusy}>
        {t.selectGbaWindows}
      </button>
    {:else}
      <button on:click={() => doScan(false)} disabled={loading}>
        {loading ? t.scanning : t.refresh}
      </button>
      <label class="chk">
        <input type="checkbox" bind:checked={gbaOnly} />
        {t.gbaOnly}
      </label>
    {/if}
    <span class="sep"></span>
    <label class="chk">
      {t.mode}:
      <select bind:value={streamMode}>
        <option value="WebrtcPlus">{t.webrtcPlus}</option>
        <option value="Webrtc">{t.webrtc}</option>
        <option value="MJPEG">{t.mjpeg}</option>
        <option value="WebrtcVp9">{t.webrtcVp9}</option>
      </select>
    </label>
    <label class="chk">
    {t.controller}:
      <select bind:value={controller} on:change={selectController}>
        <option value={0}>{t.controllerNone}</option>
        <option value={1}>{t.controllerUsb}</option>
        <option value={2}>{t.controllerVirtual}</option>
      </select>
    </label>
    <span class="sep"></span>
    {#if isWayland}
      <span class="info">{t.waylandTitle}</span>
    {:else}
      <span class="info">{t.autoScan}</span>
    {/if}
    <span class="grow"></span>
    <label class="chk">
      {t.language}:
      <select bind:value={lang}>
        <option value="it">Italiano</option>
        <option value="en">English</option>
        <option value="es">Español</option>
      </select>
    </label>
  </div>

  {#if error}
    <div class="error-bar">{error}</div>
  {/if}

  <div class="table-wrap">
    {#if isWayland}
      {#if waylandSources.length === 0}
        <div class="wayland-hint">{t.waylandHintBefore}<span class="accent-text">{t.waylandHintAction}</span>{t.waylandHintAfter}</div>
      {/if}
      <table>
        <thead>
          <tr>
            <th style="width:50px">{t.slot}</th>
            <th>{t.sourceLabel}</th>
            <th style="width:120px">{t.assignSlot}</th>
            <th style="width:340px">{t.status}</th>
          </tr>
        </thead>
        <tbody>
          {#each waylandSources as src (src.id)}
            <tr class:gba={src.gba_slot}>
              <td class="slot-cell">
                {#if src.gba_slot}
                  <span class="slot-badge">GBA{src.gba_slot}</span>
                {:else}
                  <span class="slot-badge empty">—</span>
                {/if}
              </td>
              <td class="title-cell" title={src.label}>{src.label}</td>
              <td>
                <select
                  value={src.gba_slot != null ? String(src.gba_slot) : ""}
                  on:change={(e) => assignWaylandSlot(src, /** @type {HTMLSelectElement} */(e.currentTarget).value)}
                >
                  <option value="">—</option>
                  <option value="1">GBA1</option>
                  <option value="2">GBA2</option>
                  <option value="3">GBA3</option>
                  <option value="4">GBA4</option>
                </select>
              </td>
              <td>
                {#if src.gba_slot}
                  {#if streams[src.gba_slot]}
                    {@const s = streams[src.gba_slot]}
                    {@const url = streamViewUrl(src.gba_slot, s.mode)}
                    <span class="mode-badge" class:webrtc={s.mode === 'Webrtc' || s.mode === 'WebrtcPlus' || s.mode === 'WebrtcVp9'}>
                      {modeLabel(s.mode)}
                    </span>
                    <code class="url" on:click={() => copyToClipboard(url)} title={t.clickToCopy}>
                      {url}
                    </code>
                    <button on:click={() => stopStream(src.gba_slot)}>{t.stop}</button>
                  {:else}
                    <button on:click={() => startWaylandStream(src)}>{t.startStream}</button>
                  {/if}
                {/if}
              </td>
            </tr>
          {/each}
          {#if waylandSources.length === 0}
            <tr><td colspan="4" class="empty">{t.noWaylandSourcesYet}</td></tr>
          {/if}
        </tbody>
      </table>
    {:else}
      <table>
        <thead>
          <tr>
            <th style="width:50px">{t.slot}</th>
            <th>{t.windowTitle}</th>
            <th style="width:80px">PID</th>
            <th style="width:90px">{t.size}</th>
            <th style="width:380px">{t.status}</th>
          </tr>
        </thead>
        <tbody>
          {#each windows as w (w.hwnd)}
            <tr class:gba={w.gba_slot}>
              <td class="slot-cell">
                {#if w.gba_slot}<b>GBA{w.gba_slot}</b>{/if}
              </td>
              <td class="title-cell" title={w.title}>{w.title}</td>
              <td class="mono">{w.pid}</td>
              <td class="mono">{w.width}×{w.height}</td>
              <td>
                {#if w.gba_slot}
                  {#if streams[w.gba_slot]}
                    {@const s = streams[w.gba_slot]}
                    {@const url = streamViewUrl(w.gba_slot, s.mode)}
                    <span class="mode-badge" class:webrtc={s.mode === 'Webrtc' || s.mode === 'WebrtcPlus' || s.mode === 'WebrtcVp9'}>
                      {modeLabel(s.mode)}
                    </span>
                    <code class="url" on:click={() => copyToClipboard(url)} title={t.clickToCopy}>
                      {url}
                    </code>
                    <button on:click={() => stopStream(w.gba_slot)}>{t.stop}</button>
                  {:else}
                    <button on:click={() => startStream(w)}>{t.startStream}</button>
                  {/if}
                {/if}
              </td>
            </tr>
          {/each}
          {#if windows.length === 0}
            <tr><td colspan="5" class="empty">{t.noWindows}</td></tr>
          {/if}
        </tbody>
      </table>
    {/if}
  </div>

  <div class="statusbar">
    {#if isWayland}
      <span>{waylandSources.length} {t.sourceLabel}</span>
    {:else}
      <span>{windows.length} {t.windowsLabel}</span>
    {/if}
    <span class="sep-v"></span>
    <span>{Object.keys(streams).length} {t.activeStreams}</span>
    <span class="grow"></span>
    <span>{server.interfaces.length} {t.interfaces}</span>
  </div>
</div>

<style>
  :global(html), :global(body) {
    margin: 0;
    padding: 0;
    background: #f3f3f3;
    font-family: "Segoe UI", sans-serif;
    font-size: 12px;
    color: #000;
    user-select: none;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .toolbar {
    background: #fff;
    border-bottom: 1px solid #e0e0e0;
    padding: 4px 10px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sep {
    width: 1px;
    height: 16px;
    background: #e0e0e0;
  }

  .grow {
    flex: 1;
  }

  .info {
    color: #666;
    font-style: italic;
  }

  .chk {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }

  .toolbar select {
    font-family: "Segoe UI", sans-serif;
    background: #fff;
    border: 1px solid #ccc;
    padding: 2px 4px;
  }

  .server-row {
    background: #fff;
    border-bottom: 1px solid #e0e0e0;
    padding: 6px 10px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .server-row label {
    font-weight: 600;
  }

  .server-row select,
  .server-row code,
  .url {
    font-family: "Consolas", monospace;
    background: #fff;
    border: 1px solid #ccc;
    padding: 2px 6px;
    cursor: pointer;
    height: 24px;
    box-sizing: border-box;
  }

  .server-row code {
    display: inline-flex;
    align-items: center;
  }

  .server-row code:hover,
  .url:hover {
    border-color: #0078d7;
    background: #e5f1fb;
  }

  .error-bar {
    background: #fde7e9;
    border-bottom: 1px solid #c42b1c;
    color: #c42b1c;
    padding: 4px 10px;
    font-family: "Consolas", monospace;
  }

  .table-wrap {
    flex: 1;
    overflow: auto;
    background: #fff;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    background: #f9f9f9;
    border-bottom: 1px solid #e0e0e0;
    padding: 6px 10px;
    text-align: left;
    font-weight: 600;
  }

  td {
    border-bottom: 1px solid #f0f0f0;
    padding: 5px 10px;
  }

  tbody tr:hover {
    background: #e5f1fb;
  }

  tr.gba {
    background: #fff8e1;
    font-size: 15px;
    font-weight: 600;
    padding: 10px;
  }

  tr.gba:hover {
    background: #ffecb3;
  }

  .slot-cell {
    text-align: center;
  }

  .title-cell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 0;
  }

  .mono {
    font-family: "Consolas", monospace;
  }

  .empty {
    text-align: center;
    color: #888;
    padding: 16px;
    font-style: italic;
  }

  .mode-badge {
    display: inline-block;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 3px;
    background: #e0e0e0;
    color: #555;
    vertical-align: middle;
  }

  .mode-badge.webrtc {
    background: #d0e8ff;
    color: #0050a0;
  }

  button {
    background: #fff;
    border: 1px solid #999;
    padding: 4px 12px;
    font-family: "Segoe UI", sans-serif;
    font-size: 12px;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    background: #e6e6e6;
    border-color: #0078d7;
  }

  button:active:not(:disabled) {
    background: #cccccc;
  }

  button:disabled {
    color: #aaa;
    cursor: not-allowed;
    background: #f5f5f5;
  }

  .statusbar {
    background: #f3f3f3;
    border-top: 1px solid #e0e0e0;
    padding: 4px 10px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: #555;
  }

  .sep-v {
    width: 1px;
    height: 12px;
    background: #d0d0d0;
  }

  .wayland-hint {
    background: #fff8e1;
    border-bottom: 1px solid #f0d878;
    padding: 6px 10px;
    color: #555;
    font-style: italic;
  }

  .wayland-banner {
    background: #fff8e1;
    border-bottom: 2px solid #f9a825;
    padding: 8px 12px;
    font-size: 13px;
    letter-spacing: 0.2px;
  }

  .wayland-alert {
    color: #d32f2f;
    font-weight: 700;
    text-transform: uppercase;
  }

  .wayland-text {
    color: #4a3400;
    font-weight: 600;
  }

  .wayland-slot {
    color: #bf360c;
    font-weight: 700;
  }

  .primary-btn {
    background: #0078d7;
    border: 1px solid #005a9e;
    color: #fff;
    padding: 6px 18px;
    font-size: 14px;
    font-weight: 600;
    box-shadow: 0 1px 3px rgba(0,120,215,0.35);
    transition: background 0.1s ease, box-shadow 0.1s ease;
  }

  .primary-btn:hover:not(:disabled) {
    background: #106ebe;
    border-color: #005a9e;
    box-shadow: 0 2px 6px rgba(0,120,215,0.45);
  }

  .primary-btn:active:not(:disabled) {
    background: #005a9e;
    box-shadow: inset 0 1px 2px rgba(0,0,0,0.2);
  }

  .slot-badge {
    display: inline-block;
    background: #0078d7;
    color: #fff;
    font-weight: 700;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 10px;
  }

  .slot-badge.empty {
    background: #e0e0e0;
    color: #888;
  }

  .accent-text {
    color: #0078d7;
    font-weight: 700;
  }

.qr-toggle {
    margin-left: auto;
}

.qr-section {
    background: #f8f8f8;
    border-bottom: 1px solid #e0e0e0;
    padding: 12px 16px 16px;
}

.qr-section-header {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 12px;
}

.qr-section-header strong {
    font-size: 14px;
}

.qr-section-header span {
    font-size: 12px;
    color: #666;
}

.qr-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(150px, 1fr));
    gap: 16px;
}

.qr-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
}

.qr-slot {
    font-weight: 600;
    font-size: 13px;
}

.qr-item img {
    width: 160px;
    height: 160px;
    background: white;
    border: 1px solid #ccc;
    padding: 6px;
    box-sizing: border-box;
}

.qr-item code {
    font-family: "Consolas", monospace;
    font-size: 10px;
    color: #555;
}

.qr-loading {
    width: 160px;
    height: 160px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #eee;
    color: #777;
}
</style>