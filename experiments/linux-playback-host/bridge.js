// Runs Diagnostics.js, shared with the macOS helper, in Spotify's page before
// Spotify's own scripts (as WebKit's atDocumentStart), and gives it the
// message channel it uses there: window.webkit.messageHandlers.probe.
// main.js reads each message from the page's console as "PROBE <json>".
'use strict';
const fs = require('fs');
const path = require('path');

const log = console.log.bind(console);
window.webkit = { messageHandlers: { probe: { postMessage: (text) => log('PROBE ' + text) } } };
// Both a packaged copy next to this file and the repository layout work.
const candidates = [
  path.join(__dirname, 'Diagnostics.js'),
  path.join(__dirname, '..', 'macos-playback-host', 'Diagnostics.js'),
];
const source = fs.readFileSync(candidates.find((file) => fs.existsSync(file)), 'utf8');
(0, eval)(source);
