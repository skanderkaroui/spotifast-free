// Spotifast Free's Linux playback helper: Spotify's unchanged web player in
// castLabs Electron (Chromium with Widevine), the Linux counterpart of the
// macOS helper (../macos-playback-host/Probe.swift). It speaks the same
// protocol: JSON lines on stdin ({"cmd":"connect"|"query"|"show-sign-in"|
// "quit"}) and the page's events, from Diagnostics.js, as JSON lines on
// stdout. Spotify plays every song and ad itself, with its own DRM.
'use strict';
const { app, components, BrowserWindow } = require('electron');
const path = require('path');
const readline = require('readline');

const PLAYER_URL = 'https://open.spotify.com/preferences';
const LOGIN_URL = 'https://accounts.spotify.com/login?continue=' + encodeURIComponent(PLAYER_URL);
const headless = process.argv.includes('--headless');

const out = (line) => process.stdout.write(line + '\n');
const note = (event) => out(JSON.stringify(event));

// The helper keeps its own Spotify sign-in, apart from any browser.
app.setPath('userData', process.env.SPOTIFAST_HELPER_PROFILE || path.join(app.getPath('appData'), 'spotifast-free-helper'));
app.setName('Spotifast Free');
// Spotify's player starts audio without a click, as in the macOS helper.
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

let win;
let pageReady = false;
const pending = [];
let askedToSignIn = false;
let userAskedSignIn = false;
let lastRecovery = 0;

function call(fn, ...args) {
  // Arguments go in as JSON data, never spliced into script text as code.
  const json = JSON.stringify(args);
  return win.webContents
    .executeJavaScript(`(async (a) => { await window.${fn}?.(...a); })(${json})`, true)
    .catch(() => {});
}

function handle(line) {
  let command;
  try { command = JSON.parse(line); } catch { return; }
  if (command.cmd === 'show-sign-in') {
    userAskedSignIn = true;
    if (askedToSignIn) showSignIn(true);
    return;
  }
  if (command.cmd === 'quit') return app.quit();
  if (!pageReady) return void pending.push(line);
  switch (command.cmd) {
    case 'connect':
      if (typeof command.action === 'string') call('__spotifastConnect', command.action, command.args || {});
      break;
    case 'query':
      if (command.id !== undefined && command.query) call('__spotifastQuery', command.id, command.query);
      break;
    case 'media':
      if (['play', 'pause', 'nexttrack', 'previoustrack'].includes(command.action)) {
        call('__spotifastMediaSession', command.action, {});
      }
      break;
    default:
  }
}

function showSignIn(onTop) {
  win.show();
  if (onTop) {
    win.setAlwaysOnTop(true);
    win.focus();
    win.once('blur', () => win.setAlwaysOnTop(false));
  }
}

function account(state) {
  if (state === 'signed-out') {
    askedToSignIn = true;
    if (!headless) return;
    if (!win.webContents.getURL().startsWith('https://accounts.spotify.com/')) win.loadURL(LOGIN_URL);
    showSignIn(userAskedSignIn);
  } else if (state === 'signed-in' && askedToSignIn) {
    askedToSignIn = false;
    userAskedSignIn = false;
    if (!headless) return app.quit();
    win.hide();
    if (!win.webContents.getURL().startsWith(PLAYER_URL)) {
      pageReady = false;
      win.loadURL(PLAYER_URL);
    }
  }
}

// Spotify dropped this page's device and could not get it back: reload the
// page, which registers it again, and replay the refused press once a minute
// at most, as the macOS helper does.
function recoverLostDevice(event) {
  if (Date.now() - lastRecovery < 60000 || typeof event.action !== 'string') return false;
  lastRecovery = Date.now();
  note({ event: 'device-recovery', action: event.action });
  pageReady = false;
  pending.unshift(JSON.stringify({ cmd: 'connect', action: event.action, args: event.args || {} }));
  win.webContents.reload();
  return true;
}

function onProbe(text) {
  let event;
  try { event = JSON.parse(text); } catch { return; }
  if (event.event === 'control' && event.how === 'device-lost' && recoverLostDevice(event)) return;
  out(text);
  if (event.event === 'account' && typeof event.state === 'string') account(event.state);
}

app.whenReady().then(async () => {
  // Widevine arrives through Chromium's component updater on first run.
  try {
    await components.whenReady();
  } catch (error) {
    note({ event: 'probe-error', reason: 'widevine-unavailable', message: String(error).slice(0, 300) });
  }
  // The macOS helper's size: narrower, Spotify drops the top bar whose
  // account markers Diagnostics.js reads.
  win = new BrowserWindow({
    width: 900,
    height: 600,
    show: false,
    title: 'Sign in to Spotify',
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'bridge.js'),
      // The preload gives Diagnostics.js the page world it runs in on macOS.
      contextIsolation: false,
      sandbox: false,
      nodeIntegration: false,
      backgroundThrottling: false,
    },
  });
  // Spotify serves Electron's user agent a reduced page without the top bar
  // whose account markers Diagnostics.js reads; as plain Chrome (the same
  // engine) it serves the full web player.
  win.webContents.setUserAgent(
    win.webContents.getUserAgent()
      .replace(/ \S+\/[\d.]+ (?=Chrome)/, ' ')
      .replace(/ Electron\/\S+/, ''));
  // Closing the sign-in window hides it: Spotifast keeps talking to the page.
  win.on('close', (e) => {
    if (headless && !app.isQuitting) { e.preventDefault(); win.hide(); }
  });
  app.on('before-quit', () => { app.isQuitting = true; });
  win.webContents.on('console-message', (_e, _level, message) => {
    if (message.startsWith('PROBE ')) onProbe(message.slice(6));
  });
  win.webContents.setWindowOpenHandler(({ url }) => {
    win.loadURL(url);
    return { action: 'deny' };
  });
  win.webContents.on('did-finish-load', () => {
    if (pageReady) return;
    // Spotify's player registers its controls a moment after the page loads.
    setTimeout(() => {
      pageReady = true;
      pending.splice(0).forEach(handle);
    }, 3000);
  });
  win.webContents.on('did-fail-load', (_e, code, description, url, mainFrame) => {
    if (mainFrame) note({ event: 'navigation-error', code, description });
  });
  if (!headless) win.show();
  win.loadURL(PLAYER_URL);
  readline.createInterface({ input: process.stdin }).on('line', handle).on('close', () => app.quit());
});

app.on('window-all-closed', () => app.quit());
