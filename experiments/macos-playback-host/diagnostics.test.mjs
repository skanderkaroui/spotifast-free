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
