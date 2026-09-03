// Tiny client: open WS once user taps the button, find a gamepad (browsers
// only expose one after a button press on the page), poll at rAF rate, send
// only when the serialized state changes. WS URL uses location.host so the
// port matches whatever served the viewer (8080 today).

const SLOT = {slot};
let ws = null,
    gpIndex = null;
let prevBuf = null;
let gpHideTimer = null;
let gpCycleTimer = null;
let gpDismissed = false;
let pollTimer = null;
let detectTimer = null;
const cycleTexts = ['🎮 Connect Controller', '🎮 Press any button'];
let cycleIdx = 0;

function getToast() {
    return document.getElementById('gp-toast');
}

function getText() {
    return document.getElementById('gp-text');
}

function setToastText(text) {
    const span = getText();
    if (span) span.textContent = text;
}

function showToast(autoHide) {
    if (gpDismissed) return;
    const t = getToast();
    if (!t) return;
    t.classList.add('visible');
    if (gpHideTimer) {
        clearTimeout(gpHideTimer);
        gpHideTimer = null;
    }
    if (autoHide) {
        gpHideTimer = setTimeout(() => {
            hideToast();
        }, 3000);
    }
}

function hideToast() {
    const t = getToast();
    if (t) {
        t.classList.remove('visible');
        t.classList.remove('connected');
    }
    if (gpHideTimer) {
        clearTimeout(gpHideTimer);
        gpHideTimer = null;
    }
}

function startCycle() {
    if (gpDismissed) return;
    if (gpCycleTimer) {
        clearTimeout(gpCycleTimer);
    }
    setToastText(cycleTexts[cycleIdx]);
    cycleIdx = (cycleIdx + 1) % cycleTexts.length;
    showToast(false);
    gpCycleTimer = setTimeout(startCycle, 3000);
}

function stopCycle() {
    if (gpCycleTimer) {
        clearTimeout(gpCycleTimer);
        gpCycleTimer = null;
    }
}

function stopTimers() {
    if (pollTimer) {
        clearTimeout(pollTimer);
        pollTimer = null;
    }
    if (detectTimer) {
        clearTimeout(detectTimer);
        detectTimer = null;
    }
}

function dismissGamepad() {
    gpDismissed = true;
    if (ws) {
        try {
            ws.close();
        } catch (e) {}
        ws = null;
    }
    gpIndex = null;
    prevBuf = null;
    stopTimers();
    stopCycle();
    hideToast();
}

function isGpConnected() {
    return ws && ws.readyState === 1 && gpIndex !== null;
}

function refresh() {
    if (gpDismissed) return;
    const t = getToast();
    const open = ws && ws.readyState === 1;
    if (gpIndex !== null && open) {
        stopCycle();
        if (t) t.classList.add('connected');
        setToastText('🎮 Connected');
        showToast(true);
    } else if (open) {
        stopCycle();
        if (t) t.classList.remove('connected');
        setToastText('🎮 Press any button');
        showToast(false);
    } else {
        if (t) t.classList.remove('connected');
        startCycle();
    }
}

function connectGamepad() {
    if (gpDismissed) return;
    if (ws && (ws.readyState === 0 || ws.readyState === 1)) return;
    try {
        ws = new WebSocket('ws://' + location.host + '/ws/' + SLOT);
    } catch (e) {
        return;
    }
    ws.onopen = () => {
        refresh();
        stopTimers();
        poll();
    };
    ws.onclose = () => {
        ws = null;
        gpIndex = null;
        prevBuf = null;
        refresh();
        autoDetectGamepad();
    };
    ws.onerror = () => {
        try {
            ws && ws.close();
        } catch (e) {}
    };
}
window.addEventListener('gamepadconnected', e => {
    gpIndex = e.gamepad.index;
    if (!ws || ws.readyState !== 1) connectGamepad();
    refresh();
});
window.addEventListener('gamepaddisconnected', e => {
    if (e.gamepad.index === gpIndex) {
        gpIndex = null;
        prevBuf = null;
        refresh();
    }
});

function bufEqual(a, b) {
    if (!a || !b || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++)
        if (a[i] !== b[i]) return false;
    return true;
}

function poll() {
    if (!ws || ws.readyState !== 1) return;
    if (gpIndex === null) {
        const pads = navigator.getGamepads();
        for (let i = 0; i < pads.length; i++) {
            if (pads[i]) {
                gpIndex = i;
                refresh();
                break;
            }
        }
    }
    const gp = gpIndex !== null ? navigator.getGamepads()[gpIndex] : null;
    if (gp) {
        const buf = new Uint8Array(32);
        const dv = new DataView(buf.buffer);
        for (let i = 0; i < 4; i++) {
            dv.setFloat32(i * 4, gp.axes[i] || 0, true);
        }
        for (let i = 0; i < 16; i++) {
            buf[16 + i] = gp.buttons[i] && gp.buttons[i].pressed ? 1 : 0;
        }
        if (!bufEqual(buf, prevBuf)) {
            ws.send(buf);
            prevBuf = buf;
        }
    }
    pollTimer = setTimeout(poll, 8);
}

function autoDetectGamepad() {
    if (gpDismissed) return;
    if (!ws || ws.readyState !== 1) {
        const pads = navigator.getGamepads();
        for (let i = 0; i < pads.length; i++) {
            if (pads[i]) {
                gpIndex = i;
                connectGamepad();
                break;
            }
        }
    }
    detectTimer = setTimeout(autoDetectGamepad, 8);
}
autoDetectGamepad();
startCycle();