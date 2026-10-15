// Checks that every open.spotify.com protocol fact Diagnostics.js relies on
// still exists in Spotify's own web player bundles, and that every path
// Diagnostics.js sends appears there. Run after re-downloading the bundles:
//   SPOTIFY_BUNDLES=/path/to/bundles node rea-conformance.mjs
// Exits non-zero on any FAIL.
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';

const dir = process.env.SPOTIFY_BUNDLES;
if (!dir) {
  console.error('Set SPOTIFY_BUNDLES to a folder of open.spotify.com bundles (vendor~web-player.*.js, web-player.*.js, chunks/).');
  process.exit(2);
}
const list = async (sub, re) => (await readdir(join(dir, sub)).catch(() => []))
  .filter((name) => re.test(name)).map((name) => join(dir, sub, name));
const vendorFiles = await list('.', /^vendor~web-player\.[0-9a-f]+\.js$/);
const appFiles = await list('.', /^web-player\.[0-9a-f]+\.js$/);
const chunkFiles = await list('chunks', /\.js$/);
const load = async (files) => (await Promise.all(files.map((f) => readFile(f, 'utf8')))).join('\n');
const vendor = await load(vendorFiles);
const app = await load(appFiles);
const chunks = await load(chunkFiles);
const all = [vendor, app, chunks].join('\n');
const page = await readFile(new URL('Diagnostics.js', import.meta.url), 'utf8');

let failed = 0;
const line = (ok, name, note = '') => {
  if (!ok) failed += 1;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${note ? `  (${note})` : ''}`);
};
const has = (source, pattern) => (typeof pattern === 'string' ? source.includes(pattern) : pattern.test(source));
const check = (name, source, ...patterns) => line(patterns.every((p) => has(source, p)), name);

if (!vendorFiles.length || !appFiles.length) {
  line(false, `bundles found in ${dir}`, 'need vendor~web-player.*.js and web-player.*.js');
  process.exit(1);
}
console.log(`bundles: ${[...vendorFiles, ...appFiles].map((f) => f.split('/').pop()).join(', ')} + ${chunkFiles.length} chunks\n`);

console.log('-- Spotify still has what Diagnostics.js relies on');
// Connect API client (vendor): play(), pause(), resume(), skipNext()..., addToQueue().
for (const endpoint of ['play', 'pause', 'resume', 'skip_next', 'skip_prev', 'seek_to', 'set_shuffling_context', 'set_options', 'add_to_queue']) {
  check(`_sendPlayerCommand("${endpoint}")`, vendor, `_sendPlayerCommand("${endpoint}"`);
}
// _makeEndpoint: `${this._endpoint}/${t}/${e}/from/${o}/to/${n}`.
check('_makeEndpoint url format .../from/<id>/to/<id>', vendor,
  /_makeEndpoint\(\w,\w,\w\)\{[^}]*`\$\{this\._endpoint\}\/\$\{\w\}\/\$\{\w\}\/from\/\$\{\w\}\/to\/\$\{\w\}`/);
// _sendPlayerCommand: _makeEndpoint("v1","player/command",...) with POST.
check('_sendPlayerCommand -> _makeEndpoint("v1","player/command") POST', vendor,
  /_makeEndpoint\("v1","player\/command",[^)]*\)\}\)\.then\(\w=>this\._sendRequest\(\w+\.I\.POST/);
// Connect client constructor: default endpoint @webgate/connect-state.
check('connect endpoint "@webgate/connect-state"', vendor, '"@webgate/connect-state"');
// _sendConnectCommand: "volume" is PUT, url connect/${t}.
check('_sendConnectCommand("volume") uses PUT', vendor, /_sendConnectCommand\(\w,\w=null,\w=\{\}\)\{let \w="volume"===\w\?\w+\.I\.PUT/);
check('_sendConnectCommand -> _makeEndpoint("v1",`connect/${t}`)', vendor, /_makeEndpoint\("v1",`connect\/\$\{\w\}`/);
// setVolume: Math.round(65535*t).
check('volume scale Math.round(65535*t)', vendor, /let \w=Math\.round\(65535\*\w\);return this\._sendConnectCommand\("volume"/);
// _generatePlayOptions.
check('_generatePlayOptions license', vendor, /_generatePlayOptions\(\w,\w\)\{var \w;let \w=\{license:/);
check('_generatePlayOptions skip_to{track_uid,track_index,track_uri,page_index}', vendor,
  /skip_to=\{track_uid:\w\.trackUID,track_index:\w\.index,track_uri:\w\.trackURI,page_index:\w\.pageIndex\}/);
check('_generatePlayOptions seek_to', vendor, /\.seek_to=\w\.initialOffset/);
check('_generatePlayOptions initially_paused', vendor, /\.initially_paused=\w\.paused/);
check('_generatePlayOptions player_options_override{shuffling_context}', vendor,
  /\.player_options_override=\{\};let \w=\w\.player_options_override;[^}]*\.shuffling_context=/);
check('player_options_override repeating_context / repeating_track', vendor,
  /player_options_override[\s\S]{0,600}repeating_context[\s\S]{0,300}repeating_track/);
// playContext: {uri:t,url:`context://${t}`,metadata:...}.
check('play context {uri, url:`context://${t}`, metadata}', vendor, /\{uri:\w,url:`context:\/\/\$\{\w\}`,metadata:/);
// playPages / playTracks fallback context.
check('playPages uri "spotify:internal:harmony-play-tracks" with pages:[{tracks}]', vendor,
  /"spotify:internal:harmony-play-tracks",metadata:[^}]*\|\|\{\},pages:\[\{tracks:\w\}\]/);
// Go to radio (web-player): seed_to_playlist request and response.
check('seed_to_playlist path `/seed_to_playlist/${t}`', app, /withPath\(`\/seed_to_playlist\/\$\{\w\}`\)/);
check('seed_to_playlist response-format=json', app, /seed_to_playlist\/\$\{\w\}`\)\.withQueryParameters\(\{"response-format":"json"\}\)/);
check('seed_to_playlist host is inspiredby-mix/v2', app, '/inspiredby-mix/v2`');
check('radio reads body.mediaItems[0].uri', app, /\.body\.mediaItems\[0\]/);
// Restrictions read by the web player's now-playing state.
for (const key of ['disallow_skipping_next_reasons', 'disallow_skipping_prev_reasons', 'disallow_seeking_reasons',
  'disallow_toggling_shuffle_reasons', 'disallow_toggling_repeat_context_reasons', 'disallow_toggling_repeat_track_reasons',
  'disallow_add_to_queue_reasons']) {
  check(`restrictions.${key}`, all, `${key}:`);
}
// Connect client _register: observer id "hobs_"+id cut to 40 chars.
check('_register observer id ("hobs_"+id).replace(er,"$1")', vendor, /\("hobs_"\+this\._localDeviceId\)\.replace\((\w+),"\$1"\)/);
check('er = /^([a-zA-Z0-9_%:-]{1,40}).*$/', vendor, 'er=/^([a-zA-Z0-9_%:-]{1,40}).*$/');
check('connect-state/v1/cluster', vendor, /connect-state\\\/v1\\\/cluster\$|\/v1\/cluster`/);
// Track playback state reports: PUT ${endpoint}/v1/devices/${id}/state.
check('track-playback state PUT .../v1/devices/<id>/state', vendor,
  /`\$\{this\._endpoint\}\/v1\/devices\/\$\{this\._deviceId\}\/state`;[^;]*this\._transport\.request\(\w,\{authorize:!0,method:"PUT"/);
check('track-playback endpoint "@webgate/track-playback"', all, '@webgate/track-playback');
// Server time: melody/v1/time.
check('server time "@webgate/melody/v1/time"', vendor, '"@webgate/melody/v1/time"');
// _offsetPosition(t,e,n=1,o): e+((o||now)-t)*n, floored at 0.
check('_offsetPosition = pos + (now - timestamp) * speed, >= 0', vendor,
  /_offsetPosition\((\w),(\w),(\w)=1,(\w)\)\{let (\w)=\2\+\(\(\4\|\|this\._serverTime\.getApproximate\(\)\)-\1\)\*\3;return \5<0\?0:\5\}/);
check('_offsetPosition called with timestamp, position_as_of_timestamp, playback_speed', vendor,
  /this\._offsetPosition\(\w\.timestamp,\w\.position_as_of_timestamp,\w\.playback_speed/);
// Queue manager: next_tracks feeds the queue.
check('queue built from next_tracks', vendor, /next_tracks:\w\.tracks/);
// Now-playing bar (dwp-now-playing-bar chunk).
check('ad marker data-testid "ad-controls"', chunks, '"data-testid":"ad-controls"');
// Canvas player: playTracks(..., {muted:!0, loop:!0}).
check('Canvas playTracks({muted:!0,loop:!0})', all, /playTracks\(\[\w\],\{[^}]*muted:!0,loop:!0/);

console.log('\n-- Every path Diagnostics.js sends appears in Spotify\'s bundles');
// Pull the URL templates out of Diagnostics.js and normalise ${...} to a wildcard.
const sentPaths = new Set();
// base() is `https://${connect.host}/connect-state/v1`; inline it first.
const expanded = page.replaceAll('${base()}', '/connect-state/v1');
for (const m of expanded.matchAll(/`([^`\n]*\/(?:v1|v2)\/[^`\n]*)`/g)) {
  if (m[1].endsWith('/connect-state/v1')) continue;
  const path = m[1].replace(/^https:\/\/[^/]+/, '')
    .replace(/^\$\{[^}]+\}/, '').replace(/\?.*$/, '');
  sentPaths.add(path);
}
const endpoints = [...page.matchAll(/send\('([a-z_]+)'/g)].map((m) => m[1]);
const rows = [];
// Each sent path, split into its literal segments: each must appear in Spotify's code.
const pathEvidence = {
  '/connect-state/v1/player/command/from/*/to/*': [/@webgate\/connect-state"/, /"player\/command"/, /\/from\/\$\{\w\}\/to\/\$\{\w\}/],
  '/connect-state/v1/connect/volume/from/*/to/*': [/@webgate\/connect-state"/, /`connect\/\$\{\w\}`/, /_sendConnectCommand\("volume"/],
  '/connect-state/v1/cluster': [/connect-state\\\/v1\\\/cluster/, /\/v1\/cluster`/],
  '/melody/v1/time': ['@webgate/melody/v1/time'],
  '/inspiredby-mix/v2/seed_to_playlist/*': ['/inspiredby-mix/v2`', '/seed_to_playlist/${'],
  '/track-playback/v1/devices/*/state': ['@webgate/track-playback', /\/v1\/devices\/\$\{this\._deviceId\}\/state`/],
  '/connect-state/v1/devices/hobs_*': [/"hobs_"\+this\._localDeviceId/, /\/v1\/devices\/\$\{/],
};
const normalised = new Set();
for (const p of sentPaths) {
  const n = p.replace(/\$\{[^}]+\}/g, '*').replace(/\\\//g, '/');
  if (/connect-state\/v1\/devices\/hobs_/.test(n) || /^\/connect-state\\?\/v1\\?\/devices\\?\/hobs_/.test(p)) normalised.add('/connect-state/v1/devices/hobs_*');
  else if (/track-playback/.test(n)) normalised.add('/track-playback/v1/devices/*/state');
  else normalised.add(n);
}
// The learn() regexes are not template strings; include what they match.
if (page.includes('connect-state\\/v1\\/devices\\/hobs_')) normalised.add('/connect-state/v1/devices/hobs_*');
if (page.includes('track-playback\\/v1\\/devices\\/')) normalised.add('/track-playback/v1/devices/*/state');
for (const path of [...normalised].sort()) {
  const evidence = pathEvidence[path];
  const ok = !!evidence && evidence.every((p) => has(all, p));
  rows.push({ kind: 'path', value: path, ok, note: evidence ? '' : 'no evidence rule: add one' });
}
for (const endpoint of [...new Set(endpoints)].sort()) {
  rows.push({ kind: 'command', value: endpoint, ok: has(vendor, `_sendPlayerCommand("${endpoint}"`) });
}
const width = Math.max(...rows.map((r) => r.value.length));
console.log(`${'kind'.padEnd(8)}  ${'sent by Diagnostics.js'.padEnd(width)}  in Spotify`);
for (const r of rows) line(r.ok, `${r.kind.padEnd(8)}  ${r.value.padEnd(width)}`, r.note);

console.log(`\n${failed ? `FAILED: ${failed} check(s)` : 'ALL PASS'}`);
process.exit(failed ? 1 : 0);
