// Runs Diagnostics.js against a fake page, without a browser or Spotify,
// and checks what it reports to Spotifast and what it sends to Spotify.
// The fake follows what open.spotify.com does (REA, vendor~web-player and
// a recorded load): its requests to gew1-spclient carry its sign-in, it
// reports its playing device with PUT .../track-playback/v1/devices/<id>/state,
// and Connect commands go to .../connect-state/v1/player/command/from/<id>/to/<id>.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

const script = await readFile(new URL('Diagnostics.js', import.meta.url), 'utf8');

class FakeMedia extends EventTarget {
  constructor({ muted = false, loop = false, duration = 200 } = {}) {
    super();
    Object.assign(this, { muted, loop, duration, currentTime: 0, error: null, paused: false });
  }
}

const shown = {};
const document = {
  createElement: () => ({}),
  querySelector: (selector) => shown[selector.match(/^\[data-testid="([^"]+)"\]$/)?.[1]] ?? null,
  querySelectorAll: () => [],
};

const device = 'a'.repeat(40);
const fakeClockStart = Date.now();
const sent = [];
let cluster = {
  timestamp: '1000000',
  active_device_id: device,
  devices: { [device]: { name: 'Web Player (Safari)', license: 'on-demand' } },
  player_state: {
    timestamp: '999000',
    position_as_of_timestamp: '30000',
    duration: '200000',
    is_paused: false,
    playback_speed: 1,
    context_uri: 'spotify:playlist:p',
    options: { shuffling_context: false, repeating_context: true, repeating_track: false },
    restrictions: { disallow_resuming_reasons: ['not_paused'], disallow_skipping_prev_reasons: ['no_prev_track'], disallow_seeking_reasons: [] },
    next_tracks: [{ uri: 'spotify:track:n1' }, { uri: 'spotify:delimiter' }, { uri: 'spotify:track:n2', metadata: { hidden: 'true' } }, { uri: 'spotify:track:n3' }],
    track: { uri: 'spotify:track:t', metadata: { title: 'Song', artist_name: 'A', artist_uri: 'spotify:artist:a', 'artist_name:1': 'B', album_title: 'Album', image_large_url: 'L', image_small_url: 'S' } },
  },
};
const pageFetch = async (input, init = {}) => {
  const url = typeof input === 'string' ? input : input.url;
  sent.push({ url, method: init.method ?? 'GET', headers: init.headers ?? {}, body: init.body ? JSON.parse(init.body) : null });
  if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => cluster };
  // Spotify's clock: 2 s after the state's timestamp.
  if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: 1001000 + (Date.now() - fakeClockStart) }) };
  if (url.includes('/inspiredby-mix/v2/seed_to_playlist/')) return { ok: true, status: 200, json: async () => ({ mediaItems: [{ uri: 'spotify:playlist:station' }] }) };
  return { ok: true, status: 200, json: async () => ({}) };
};

const posted = [];
const intervals = [];
const context = {
  window: { webkit: { messageHandlers: { probe: { postMessage: (text) => posted.push(JSON.parse(text)) } } }, fetch: pageFetch },
  location: { origin: 'https://open.spotify.com', href: 'https://open.spotify.com/' },
  navigator: {},
  console: { error() {}, warn() {} },
  document,
  Document: { prototype: document },
  HTMLMediaElement: FakeMedia,
  XMLHttpRequest: class { open() {} setRequestHeader() {} send() {} },
  Headers,
  URL,
  JSON,
  Object,
  Number,
  Math,
  Promise,
  crypto: { randomUUID: () => '0123456789abcdef0123456789abcdef' },
  setInterval: (fn) => intervals.push(fn),
  setTimeout: (fn) => { fn(); return 0; },
  addEventListener: () => {},
};
context.window.top = context.window;
context.window.window = context.window;
context.self = context.window;
runInNewContext(`var window = this.window; window.top = window; ${script}`, context);
const take = (event) => {
  const found = posted.filter((item) => item.event === event);
  return found;
};

console.log('ok: script loads on the Spotify page');

// 2. The page's own requests teach it the sign-in and its device id.
await context.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', {
  headers: { authorization: 'Bearer page-token', 'client-token': 'ct' },
});
await context.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${device}/state`, {
  method: 'PUT', headers: { authorization: 'Bearer page-token' },
});
sent.length = 0;

// 3. Play a playlist from a chosen song: one Connect "play" to itself.
await context.window.__spotifastConnect('play', { contextUri: 'spotify:playlist:p', trackUri: 'spotify:track:t', repeat: 'off' });
const play = sent.find((request) => request.url.includes('/player/command/'));
assert.ok(play, 'a Connect command was sent');
assert.equal(play.method, 'POST');
assert.equal(play.url, `https://gew1-spclient.spotify.com/connect-state/v1/player/command/from/${device}/to/${device}`);
assert.equal(play.headers.authorization, 'Bearer page-token');
assert.equal(play.headers['client-token'], 'ct');
assert.equal(play.body.command.endpoint, 'play');
assert.equal(play.body.command.context.uri, 'spotify:playlist:p');
assert.equal(play.body.command.options.skip_to.track_uri, 'spotify:track:t');
assert.equal(play.body.command.options.license, 'on-demand');
assert.deepEqual(play.body.command.options.player_options_override, { repeating_context: false, repeating_track: false });
const control = take('control').at(-1);
assert.equal(control.handled, true);
console.log('ok: play goes to the page\'s own device with its own sign-in');

// 4. Spotify's state is reported: real URI, details, shuffle and repeat.
const state = take('state').at(-1);
assert.equal(state.uri, 'spotify:track:t');
assert.equal(state.title, 'Song');
assert.deepEqual(state.artists.map((artist) => artist.name), ['A', 'B']);
assert.equal(state.repeat, 'context');
assert.equal(state.shuffle, false);
assert.deepEqual({ ...state.restrictions }, { skipNext: false, skipPrev: true, seek: false, shuffle: false, repeat: false, queue: false },
  'held reasons count, "not_paused" does not');
assert.deepEqual([...state.upcoming], ['spotify:track:n1', 'spotify:track:n3'], 'upcoming songs skip delimiters and hidden items');
const progress = take('progress').at(-1);
assert.equal(progress.durationMs, 200000);
assert.ok(Math.abs(progress.positionMs - 32000) < 200, `position moved on by Spotify's clock, not the cluster's (${progress.positionMs})`);
console.log('ok: Spotify\'s state names the song');

// 5. Other controls are single Connect commands.
const endpointOf = async (action, args) => {
  sent.length = 0;
  await context.window.__spotifastConnect(action, args);
  return sent.find((request) => request.url.includes('/player/command/'))?.body.command;
};
assert.equal((await endpointOf('next')).endpoint, 'skip_next');
assert.equal((await endpointOf('previous')).endpoint, 'skip_prev');
assert.deepEqual((await endpointOf('seek', { positionMs: 61500 })).value, 61500);
assert.equal((await endpointOf('shuffle', { on: true })).endpoint, 'set_shuffling_context');
const repeat = await endpointOf('repeat', { mode: 'track' });
assert.equal(repeat.endpoint, 'set_options');
assert.equal(repeat.repeating_track, true);
assert.equal((await endpointOf('pause')).endpoint, 'pause');
assert.equal((await endpointOf('resume')).endpoint, 'resume');
const queued = await endpointOf('queue', { uri: 'spotify:track:q' });
assert.equal(queued.endpoint, 'add_to_queue');
assert.equal(queued.track.uri, 'spotify:track:q');
sent.length = 0;
await context.window.__spotifastConnect('volume', { volume: 32768 });
const volume = sent.find((request) => request.url.includes('/connect/volume/'));
assert.equal(volume.method, 'PUT');
assert.equal(volume.url, `https://gew1-spclient.spotify.com/connect-state/v1/connect/volume/from/${device}/to/${device}`);
assert.equal(volume.body.volume, 32768);
sent.length = 0;
await context.window.__spotifastConnect('radio', { seed: 'spotify:track:seed' });
assert.ok(sent[0].url.startsWith('https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/spotify:track:seed'));
const radioPlay = sent.find((request) => request.url.includes('/player/command/'));
assert.equal(radioPlay.body.command.context.uri, 'spotify:playlist:station');
sent.length = 0;
await context.window.__spotifastConnect('radio', { seed: 'spotify:track:x/../y' });
assert.ok(!sent.some((request) => request.url.includes('seed_to_playlist')), 'a malformed seed is not sent');
assert.equal(take('control').at(-1).handled, false);
console.log('ok: controls are single Connect commands');

// 6. A list of songs plays as one page of tracks.
const list = await endpointOf('play', { tracks: ['spotify:track:a', 'spotify:track:b'], trackIndex: 1 });
assert.equal(list.context.uri, 'spotify:internal:harmony-play-tracks');
assert.deepEqual(list.context.pages[0].tracks.map((track) => track.uri), ['spotify:track:a', 'spotify:track:b']);
assert.equal(list.options.skip_to.track_index, 1);
console.log('ok: a song list plays as its own queue');

// 7. Ads: "ad-controls" marks one; the song is reported again after it.
const adTick = intervals[0];
shown['ad-controls'] = {};
adTick();
assert.equal(take('ad').at(-1).state, 'start');
delete shown['ad-controls'];
adTick();
assert.equal(take('ad').at(-1).state, 'end');
console.log('ok: ads start and end from ad-controls');

// 7b. Sign-in: Spotify's top bar shows "login-button" while signed out and
// "user-widget-link" once signed in. Each change is reported once.
const accountTick = intervals[1];
posted.length = 0;
accountTick();
assert.equal(take('account').length, 0, 'nothing is reported before the bar renders');
shown['login-button'] = {};
accountTick();
accountTick();
assert.deepEqual(take('account').map((e) => e.state), ['signed-out']);
delete shown['login-button'];
shown['user-widget-link'] = {};
accountTick();
assert.deepEqual(take('account').map((e) => e.state), ['signed-out', 'signed-in']);
delete shown['user-widget-link'];
console.log('ok: sign-in is detected from Spotify\'s own top bar');

// 7c. A play while signed out is held, reported as waiting, and sent once
// the page is signed in. The fake setTimeout runs at once, so let each poll
// yield to this test with a real one while the command waits.
const instant = context.setTimeout;
context.setTimeout = (fn) => setImmediate(fn);
shown['login-button'] = {};
accountTick();
posted.length = 0;
const sentBefore = sent.length;
const held = context.window.__spotifastConnect('pause');
for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setImmediate(resolve));
assert.equal(take('control').at(-1)?.how, 'signing-in', 'the command waits for sign-in');
assert.equal(sent.slice(sentBefore).filter((r) => r.url.includes('/player/command/')).length, 0, 'nothing sent while signed out');
delete shown['login-button'];
shown['user-widget-link'] = {};
accountTick();
await held;
context.setTimeout = instant;
assert.equal(take('control').at(-1)?.handled, true, 'the held command ran after sign-in');
assert.equal(sent.slice(sentBefore).filter((r) => r.url.includes('/player/command/')).length, 1, 'sent once, after sign-in');
delete shown['user-widget-link'];
console.log('ok: a command waits for sign-in and runs once signed in');

// 8. Not this account's player: no state is reported. (The pause above
// may still trigger a state read; clear what it posted first.)
await endpointOf('resume');
cluster = { ...cluster, active_device_id: 'phone' };
posted.length = 0;
await endpointOf('pause');
assert.equal(take('state').length, 0);
console.log('ok: another device\'s playback is not reported as ours');

// 9. Real recorded clusters (src/testdata/connect/*.json, each
// {own_device, cluster}): a fresh page learns its device the way the web
// player reports it (PUT .../track-playback/v1/devices/<own_device>/state),
// reads the recorded cluster, and must post the expected state.
const fixtureDir = new URL('../../src/testdata/connect/', import.meta.url);
const recorded = async (name) => JSON.parse(await readFile(new URL(name, fixtureDir), 'utf8'));
const runFixture = async ({ own_device: ownDevice, cluster: recordedCluster }) => {
  const out = [];
  const fetchFake = async (input) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => recordedCluster };
    if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: Number(recordedCluster.timestamp) }) };
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake },
    setInterval: () => {},
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer t' } });
  await page.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${ownDevice}/state`,
    { method: 'PUT', headers: { authorization: 'Bearer t' } });
  await page.window.__spotifastConnect('pause');
  const sentTo = out.find((item) => item.event === 'control');
  assert.equal(sentTo?.handled, true, 'the command reached the recorded device');
  return out.filter((item) => item.event === 'state').at(-1);
};
const noHolds = { skipNext: false, skipPrev: false, seek: false, shuffle: false, repeat: false, queue: false };
const playlist = 'spotify:playlist:playlist00000000000001';
const song = 'spotify:track:track00000000000000006';
const expected = {
  'playlist-playing.json': { uri: song, contextUri: playlist, shuffle: false, repeat: 'off', paused: false, restrictions: noHolds },
  'playlist-paused.json': { uri: song, contextUri: playlist, shuffle: false, repeat: 'off', paused: true, restrictions: noHolds },
  'repeat-context.json': { uri: song, contextUri: playlist, shuffle: false, repeat: 'context', paused: false, restrictions: noHolds },
  'shuffle-repeat-track.json': { uri: song, contextUri: playlist, shuffle: true, repeat: 'track', paused: false, restrictions: noHolds },
  'after-add-to-queue.json': { uri: song, contextUri: playlist, shuffle: false, repeat: 'off', paused: false, restrictions: noHolds,
    upcoming0: 'spotify:track:track00000000000000001' },
  'radio-first-song.json': { uri: 'spotify:track:track00000000000000010', contextUri: 'spotify:playlist:playlist00000000000002',
    shuffle: false, repeat: 'off', paused: false, restrictions: { ...noHolds, skipPrev: true } },
  'track-list.json': { uri: 'spotify:track:track00000000000000011', contextUri: 'spotify:internal:harmony-play-tracks',
    shuffle: true, repeat: 'off', paused: false, restrictions: { ...noHolds, skipPrev: true } },
};
const fixtureNames = (await readdir(fixtureDir)).filter((name) => name.endsWith('.json')).sort();
for (const name of fixtureNames) {
  const fixture = await recorded(name);
  if (!fixture.cluster?.player_state) continue; // not a cluster recording (e.g. seed-to-playlist.json)
  const want = expected[name];
  assert.ok(want, `${name}: add its expected state to this test`);
  const got = await runFixture(fixture);
  assert.ok(got, `${name}: a state was posted`);
  for (const key of ['uri', 'contextUri', 'shuffle', 'repeat', 'paused']) assert.equal(got[key], want[key], `${name}: ${key}`);
  assert.deepEqual({ ...got.restrictions }, want.restrictions, `${name}: restrictions`);
  if (want.upcoming0) assert.equal(got.upcoming[0], want.upcoming0, `${name}: upcoming[0] is the queued song`);
  console.log(`ok: recorded ${name}`);
}

// 10. Spotify forgot this page's device (its live connection dropped, as in
// the 24 Oct outage where every command got 404 or 410 until a restart).
// Spotify's own player answers a 404 by reading the cluster again
// (REA: _awaitResponseWithAck -> _fetchCluster). The page does the same:
// still listed, it sends the command once more; gone, it says so, so the
// helper can reload Spotify's page, which registers again.
{
  const outageDevice = 'b'.repeat(40);
  const out = [];
  const commands = [];
  let listed = true;
  let refuse = 1;
  const outageCluster = () => ({
    ...cluster,
    active_device_id: outageDevice,
    devices: listed ? { [outageDevice]: { name: 'Web Player (Safari)', license: 'on-demand' } } : {},
  });
  const fetchFake = async (input, init = {}) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => outageCluster() };
    if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: 1000000 }) };
    if (url.includes('/player/command/')) {
      commands.push(JSON.parse(init.body).command.endpoint);
      if (refuse > 0) { refuse -= 1; return { ok: false, status: 404, json: async () => ({}) }; }
      return { ok: true, status: 200, json: async () => ({}) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake },
    setInterval: () => {},
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer t' } });
  await page.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${outageDevice}/state`,
    { method: 'PUT', headers: { authorization: 'Bearer t' } });

  // Still listed: one refusal, then the same command again, accepted.
  await page.window.__spotifastConnect('pause');
  assert.deepEqual(commands, ['pause', 'pause'], 'retried once after re-reading the cluster');
  assert.equal(out.filter((e) => e.event === 'control').at(-1).handled, true);

  // Gone: refused, the cluster no longer lists this device, so no blind retry.
  commands.length = 0;
  out.length = 0;
  listed = false;
  refuse = 1;
  await page.window.__spotifastConnect('resume');
  assert.deepEqual(commands, ['resume'], 'no retry to a device Spotify dropped');
  const lost = out.filter((e) => e.event === 'control').at(-1);
  assert.equal(lost.handled, false);
  assert.equal(lost.how, 'device-lost');
  assert.equal(lost.action, 'resume');
  assert.deepEqual({ ...lost.args }, {}, 'the refused command carries its own arguments for the replay');
  console.log('ok: a forgotten device is retried if Spotify still lists it, and reported lost if not');
}

// 11. Quick pause then play: Spotify's player sends a command only after
// the previous one's acknowledgement arrives on its dealer connection
// (REA: _awaitResponseWithAck waits for ack_id, trackCommandAcknowledged,
// 3400 ms cap). Sent back to back, the hidden player received the pause
// after the play and ended paused (live trace, 24 Oct). The page now does
// the same: one command at a time, each held until its ack or the cap.
{
  const ackDevice = 'c'.repeat(40);
  const out = [];
  const log = [];
  const sockets = [];
  let next = 0;
  class FakeSocket extends EventTarget {
    constructor() { super(); sockets.push(this); }
  }
  const ack = (id) => {
    log.push(`ack ${id}`);
    const event = new Event('message');
    event.data = JSON.stringify({ uri: 'hm://connect-state/v1/cluster', payloads: [{ ack_id: id, cluster: {} }] });
    sockets.forEach((s) => s.dispatchEvent(event));
  };
  const fetchFake = async (input, init = {}) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => ({ ...cluster, active_device_id: ackDevice, devices: { [ackDevice]: { license: 'on-demand' } } }) };
    if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: 1000000 }) };
    if (url.includes('/player/command/')) {
      const id = `ack-${next += 1}`;
      log.push(`send ${JSON.parse(init.body).command.endpoint} ${id}`);
      return { ok: true, status: 200, json: async () => ({ ack_id: id }) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake, WebSocket: FakeSocket },
    WebSocket: FakeSocket,
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  new page.window.WebSocket('wss://dealer.spotify.com/');
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer t' } });
  await page.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${ackDevice}/state`,
    { method: 'PUT', headers: { authorization: 'Bearer t' } });

  const pause = page.window.__spotifastConnect('pause');
  const resume = page.window.__spotifastConnect('resume');
  await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual(log, ['send pause ack-1'], 'the play waits for the pause to be acknowledged');
  ack('ack-1');
  await pause;
  await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual(log, ['send pause ack-1', 'ack ack-1', 'send resume ack-2']);
  ack('ack-2');
  await resume;
  assert.deepEqual(out.filter((e) => e.event === 'control').map((e) => `${e.action}:${e.handled}`), ['pause:true', 'resume:true']);

  // An ack that never comes holds the next command only up to the cap.
  log.length = 0;
  const started = Date.now();
  await page.window.__spotifastConnect('pause');
  await page.window.__spotifastConnect('resume');
  assert.deepEqual(log, ['send pause ack-3', 'send resume ack-4']);
  assert.ok(Date.now() - started >= 3400 * 2 - 100, 'each waited for the cap');
  console.log('ok: commands go one at a time, each after the previous one is acknowledged');
}

// 12. A command whose request never answers holds the next one only up to
// the chain's limit; buttons do not all hang behind it.
{
  const hungDevice = 'd'.repeat(40);
  const out = [];
  const sentCommands = [];
  const fetchFake = async (input, init = {}) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => ({ ...cluster, active_device_id: hungDevice, devices: { [hungDevice]: { license: 'on-demand' } } }) };
    if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: 1000000 }) };
    if (url.includes('/player/command/')) {
      const endpoint = JSON.parse(init.body).command.endpoint;
      sentCommands.push(endpoint);
      if (endpoint === 'pause') return new Promise(() => {});
      return { ok: true, status: 200, json: async () => ({}) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake },
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer t' } });
  await page.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${hungDevice}/state`,
    { method: 'PUT', headers: { authorization: 'Bearer t' } });
  page.window.__spotifastConnect('pause');
  const started = Date.now();
  await page.window.__spotifastConnect('next');
  const waited = Date.now() - started;
  assert.deepEqual(sentCommands, ['pause', 'skip_next']);
  assert.ok(waited >= 6000 && waited < 8000, `next waited ${waited} ms`);
  console.log('ok: a command that hangs holds the next one only briefly');
}

// 13. The ack can arrive on the live connection before the command's own
// response is read (Spotify's "orphaned acknowledgement"); it still counts.
// And a response without an ack_id adds no wait at all.
{
  const dev = 'e'.repeat(40);
  const out = [];
  const log = [];
  const sockets = [];
  let withAck = true;
  let n = 0;
  class FakeSocket extends EventTarget { constructor() { super(); sockets.push(this); } }
  const fetchFake = async (input, init = {}) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.endsWith('/cluster')) return { ok: true, status: 200, json: async () => ({ ...cluster, active_device_id: dev, devices: { [dev]: { license: 'on-demand' } } }) };
    if (url.endsWith('/melody/v1/time')) return { ok: true, status: 200, json: async () => ({ timestamp: 1000000 }) };
    if (url.includes('/player/command/')) {
      const id = `early-${n += 1}`;
      log.push(JSON.parse(init.body).command.endpoint);
      if (!withAck) return { ok: true, status: 200, json: async () => ({}) };
      // The dealer delivers the ack first, then the response is read.
      const event = new Event('message');
      event.data = JSON.stringify({ type: 'message', uri: 'hm://connect-state/v1/cluster', payloads: [{ ack_id: id, cluster: {} }] });
      sockets.forEach((s) => s.dispatchEvent(event));
      return { ok: true, status: 200, json: async () => ({ ack_id: id }) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake, WebSocket: FakeSocket },
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  new page.window.WebSocket('wss://dealer.spotify.com/');
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer t' } });
  await page.window.fetch(`https://gew1-spclient.spotify.com/track-playback/v1/devices/${dev}/state`,
    { method: 'PUT', headers: { authorization: 'Bearer t' } });
  let started = Date.now();
  await page.window.__spotifastConnect('pause');
  await page.window.__spotifastConnect('resume');
  assert.ok(Date.now() - started < 1000, `an early ack still counts (${Date.now() - started} ms)`);
  withAck = false;
  started = Date.now();
  await page.window.__spotifastConnect('pause');
  await page.window.__spotifastConnect('resume');
  assert.ok(Date.now() - started < 1000, `no ack_id, no wait (${Date.now() - started} ms)`);
  assert.deepEqual(log, ['pause', 'resume', 'pause', 'resume']);
  console.log('ok: an early ack counts, and a response without ack_id adds no wait');
}

// A Free account's reads: the page sends the web player's own query with its
// own sign-in and hands back only Spotify's answer, under the asked id. A
// rejected token waits for the page's renewed one and goes once more.
{
  const out = [];
  const asked = [];
  let refuse = true;
  const fetchFake = async (input, init = {}) => {
    const url = typeof input === 'string' ? input : input.url;
    if (url.includes('/pathfinder/')) {
      asked.push({ url, headers: init.headers, body: JSON.parse(init.body) });
      if (refuse) {
        refuse = false;
        setTimeout(() => page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer renewed' } }), 50);
        return { ok: false, status: 401, text: async () => '' };
      }
      return { ok: true, status: 200, text: async () => '{"data":{"me":{"profile":{"username":"u"}}}}' };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  };
  const page = {
    ...context,
    document: { ...document, querySelector: (selector) => (selector.includes('user-widget-link') ? {} : null) },
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } }, fetch: fetchFake },
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer first', 'client-token': 'ct' } });
  const query = { operationName: 'profileAttributes', variables: {}, extensions: { persistedQuery: { version: 1, sha256Hash: 'h' } } };
  await page.window.__spotifastQuery(7, query);
  assert.equal(asked.length, 2, 'refused once, sent again');
  assert.equal(asked[0].url, 'https://api-partner.spotify.com/pathfinder/v2/query');
  assert.deepEqual(asked[0].body, query);
  assert.equal(asked[0].headers.authorization, 'Bearer first');
  assert.equal(asked[1].headers.authorization, 'Bearer renewed');
  const answers = out.filter((event) => event.event === 'query');
  assert.equal(answers.length, 1);
  assert.equal(answers[0].id, 7);
  assert.equal(answers[0].status, 200);
  assert.equal(JSON.parse(answers[0].body).data.me.profile.username, 'u');
  assert.ok(!JSON.stringify(answers).includes('Bearer'), 'the sign-in never leaves the page');
  console.log('ok: a Free read goes out with the page sign-in and only the answer comes back');
}

// A signed-out page still has an anonymous token. Its reads must not go out
// as the account's: the page reports itself signed out instead, so the app
// waits for the sign-in rather than reading an empty profile.
{
  const out = [];
  const asked = [];
  const shownHere = { 'login-button': {} };
  const page = {
    ...context,
    document: { ...document, querySelector: (selector) => shownHere[selector.match(/^\[data-testid="([^"]+)"\]$/)?.[1]] ?? null },
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } },
      fetch: async (input, init = {}) => { asked.push(String(input)); return { ok: true, status: 200, text: async () => '{"data":{"me":null}}', json: async () => ({}) }; } },
    setInterval: (fn) => { page.tick = fn; },
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer anonymous' } });
  page.tick?.();
  asked.length = 0;
  await page.window.__spotifastQuery(3, { operationName: 'profileAttributes' });
  assert.ok(!asked.some((url) => url.includes('/pathfinder/')), 'no read goes out signed out');
  const answer = out.find((event) => event.event === 'query');
  assert.equal(answer?.id, 3);
  assert.equal(answer?.how, 'signed-out');
  console.log('ok: a signed-out page reports itself signed out instead of reading');
}

// A Free account's playlist edits: the page posts the web player's own
// playlist-service change to exactly the playlist, rootlist or creation
// path asked, with its sign-in, and refuses any other target.
{
  const out = [];
  const asked = [];
  const page = {
    ...context,
    document: { ...document, querySelector: (selector) => (selector.includes('user-widget-link') ? {} : null) },
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } },
      fetch: async (input, init = {}) => {
        const url = typeof input === 'string' ? input : input.url;
        if (url.includes('/playlist/v2/')) {
          asked.push({ url, method: init.method, headers: init.headers, body: JSON.parse(init.body) });
          return { ok: true, status: 200, text: async () => '{"revision":"AAAA"}' };
        }
        return { ok: true, status: 200, json: async () => ({}) };
      } },
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer first' } });
  const delta = { deltas: [{ ops: [{ kind: 'UPDATE_LIST_ATTRIBUTES' }] }] };
  await page.window.__spotifastQuery(1, { playlistChanges: { create: true, body: { ops: [] } } });
  await page.window.__spotifastQuery(2, { playlistChanges: { playlist: '5cr4tchPl4yl1stF1xtur3', body: delta } });
  await page.window.__spotifastQuery(3, { playlistChanges: { rootlist: 'owner', body: delta } });
  await page.window.__spotifastQuery(4, { playlistChanges: { playlist: '../../me', body: delta } });
  // The helper hands the app's JSON true over as 1.
  await page.window.__spotifastQuery(5, { playlistChanges: { create: 1, body: { ops: [] } } });
  assert.deepEqual(asked.map((a) => a.url), [
    'https://spclient.wg.spotify.com/playlist/v2/playlist',
    'https://spclient.wg.spotify.com/playlist/v2/playlist/5cr4tchPl4yl1stF1xtur3/changes',
    'https://spclient.wg.spotify.com/playlist/v2/user/owner/rootlist/changes',
    'https://spclient.wg.spotify.com/playlist/v2/playlist',
  ]);
  assert.ok(asked.every((a) => a.method === 'POST' && a.headers.authorization === 'Bearer first'));
  assert.deepEqual(asked[1].body, delta);
  const answers = out.filter((event) => event.event === 'query');
  assert.deepEqual(answers.map((a) => [a.id, a.status]), [[1, 200], [2, 200], [3, 200], [4, 400], [5, 200]]);
  console.log('ok: playlist edits go only to the playlist service paths asked');
}

// Spotify's code page (accounts.spotify.com/login/otp) has one box per digit
// and keeps only the last character put into a box (REA, authn-web chunk 39:
// the box's onChange takes value.slice(-1); only onPaste fills every box).
// macOS fills a code from Messages or Mail into the focused box whole, so
// only its last digit stayed. The whole code goes to the page as the paste
// it handles instead.
{
  const listeners = [];
  const boxes = Array.from({ length: 6 }, () => {
    const box = { value: '', autocomplete: 'one-time-code', getAttribute: (name) => (name === 'autocomplete' ? 'one-time-code' : null), events: [] };
    box.dispatchEvent = (event) => { box.events.push(event); return true; };
    return box;
  });
  class DataTransfer { constructor() { this.data = {}; } setData(type, value) { this.data[type] = value; } getData(type) { return this.data[type === 'text' ? 'text/plain' : type] ?? ''; } }
  class ClipboardEvent { constructor(type, init) { Object.assign(this, { type, ...init }); } }
  const page = {
    ...context,
    location: { origin: 'https://accounts.spotify.com', href: 'https://accounts.spotify.com/en/login/otp' },
    window: { webkit: { messageHandlers: { probe: { postMessage: () => {} } } }, fetch: pageFetch },
    addEventListener: (type, listener, capture) => listeners.push({ type, listener, capture }),
    DataTransfer,
    ClipboardEvent,
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  const input = listeners.find((l) => l.type === 'input' && l.capture);
  assert.ok(input, 'the page listens for input before Spotify does');
  const fill = (box, value) => {
    box.value = value;
    let stopped = false;
    input.listener({ target: box, stopImmediatePropagation: () => { stopped = true; } });
    return stopped;
  };
  // AutoFill puts the whole code in the first box.
  assert.equal(fill(boxes[0], '482915'), true, "Spotify's own handler would keep one digit");
  const paste = boxes[0].events.at(-1);
  assert.equal(paste?.type, 'paste');
  assert.equal(paste.bubbles, true);
  assert.equal(paste.clipboardData.getData('text'), '482915');
  // Into a box already holding a digit: the code is the last six.
  fill(boxes[0], '7482915');
  assert.equal(boxes[0].events.at(-1).clipboardData.getData('text'), '482915');
  // One digit typed into a box is Spotify's to handle.
  assert.equal(fill(boxes[1], '4'), false);
  assert.equal(boxes[1].events.length, 0);
  console.log('ok: a code macOS fills in whole goes to every box');
}

// A Free account's playlist cover, as the web player uploads one (REA:
// image-upload/v4/playlist takes the JPEG bytes with only the sign-in and
// answers an upload token; playlist/<id>/register-image turns it into the
// picture the list's attributes name). The app hands the JPEG as base64.
{
  const out = [];
  const asked = [];
  const page = {
    ...context,
    atob: (b64) => Buffer.from(b64, 'base64').toString('binary'),
    document: { ...document, querySelector: (selector) => (selector.includes('user-widget-link') ? {} : null) },
    window: { webkit: { messageHandlers: { probe: { postMessage: (t) => out.push(JSON.parse(t)) } } },
      fetch: async (input, init = {}) => {
        const url = typeof input === 'string' ? input : input.url;
        if (url.includes('image-upload') || url.includes('register-image')) {
          asked.push({ url, method: init.method, headers: init.headers, body: init.body });
          return url.includes('image-upload')
            ? { ok: true, status: 200, text: async () => '{"uploadToken":"tok"}' }
            : { ok: true, status: 200, text: async () => '{"picture":"cGlj"}' };
        }
        return { ok: true, status: 200, json: async () => ({}) };
      } },
    setInterval: () => {},
    setTimeout,
    navigator: {},
  };
  runInNewContext(`var window = this.window; window.top = window; ${script}`, page);
  await page.window.fetch('https://gew1-spclient.spotify.com/melody/v1/time', { headers: { authorization: 'Bearer first', 'client-token': 'ct' } });
  const jpeg = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00]);
  await page.window.__spotifastQuery(1, { playlistImage: { playlist: '5cr4tchPl4yl1stF1xtur3', jpeg: jpeg.toString('base64') } });
  await page.window.__spotifastQuery(2, { playlistImage: { playlist: '../../me', jpeg: jpeg.toString('base64') } });
  assert.deepEqual(asked.map((a) => [a.method, a.url]), [
    ['POST', 'https://image-upload.spotify.com/v4/playlist'],
    ['POST', 'https://spclient.wg.spotify.com/playlist/v2/playlist/5cr4tchPl4yl1stF1xtur3/register-image'],
  ]);
  assert.deepEqual(asked[0].headers, { authorization: 'Bearer first', 'content-type': 'image/jpeg' });
  assert.deepEqual([...asked[0].body], [...jpeg]);
  assert.equal(asked[1].headers.authorization, 'Bearer first');
  assert.deepEqual(JSON.parse(asked[1].body), { uploadToken: 'tok' });
  const answers = out.filter((event) => event.event === 'query');
  assert.deepEqual(answers.map((a) => [a.id, a.status, a.body]), [[1, 200, '{"picture":"cGlj"}'], [2, 400, 'refused: not a playlist']]);
  console.log('ok: a Free playlist cover goes up as the web player sends it');
}
