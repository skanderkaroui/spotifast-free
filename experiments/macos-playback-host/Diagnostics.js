// Injected at document start in every frame of the hidden web player.
//
// Spotifast controls the page through open.spotify.com's own Spotify
// Connect endpoints, the same requests the web player sends for its own
// buttons (REA, vendor~web-player: _sendPlayerCommand, _generatePlayOptions,
// _register). They go out from inside the page with the page's own
// sign-in headers, which never leave the page. The page still plays every
// song and every ad itself; nothing is skipped, muted or blocked.
//
// Reported to Spotifast: media element events, now-playing metadata from
// the Media Session, Spotify's playback state from Connect, ads, page
// errors, and which key systems the page asks for.
(() => {
  const post = (event, detail) => {
    try {
      window.webkit.messageHandlers.probe.postMessage(
        JSON.stringify({ event, origin: location.origin, ...detail }));
    } catch (_) {}
  };
  const text = (value) => String(value?.message ?? value).slice(0, 300);
  const spotifyPage = location.origin === 'https://open.spotify.com' && window.top === window;

  addEventListener('error', (e) => post('page-error', { message: text(e.error ?? e.message) }));
  addEventListener('unhandledrejection', (e) => post('page-rejection', { message: text(e.reason) }));
  for (const level of ['error', 'warn']) {
    const original = console[level];
    console[level] = function (...args) {
      post(`console-${level}`, { message: args.map(text).join(' ').slice(0, 300) });
      return original.apply(this, args);
    };
  }

  const mediaEvents = ['play', 'playing', 'pause', 'ended', 'error', 'waiting', 'stalled', 'encrypted', 'waitingforkey', 'webkitneedkey', 'seeked', 'durationchange'];
  const report = (e) => {
    const el = e.target;
    if (!(el instanceof HTMLMediaElement)) return;
    // Spotify's Canvas clips are muted, looping videos beside the song (REA:
    // playTracks(..., {muted:true, loop:true})). They are never what is
    // heard, so their 7 s loops must not stand in for the song's progress.
    if (el.muted || el.loop) return;
    post('media', {
      type: e.type,
      positionMs: Math.round(el.currentTime * 1000),
      durationMs: Number.isFinite(el.duration) ? Math.round(el.duration * 1000) : null,
      mediaError: el.error ? el.error.code : null,
      initDataType: e.initDataType ?? null,
    });
  };
  for (const type of mediaEvents) addEventListener(type, report, true);
  // Players often keep their media element out of the document, where
  // window listeners never see its events. Watch every element as it is made.
  const watched = new WeakSet();
  const watch = (el) => {
    if (watched.has(el)) return el;
    watched.add(el);
    for (const type of mediaEvents) el.addEventListener(type, report);
    return el;
  };
  const createElement = Document.prototype.createElement;
  Document.prototype.createElement = function (name, ...rest) {
    const el = createElement.call(this, name, ...rest);
    return el instanceof HTMLMediaElement ? watch(el) : el;
  };
  if (window.Audio) {
    window.Audio = new Proxy(window.Audio, { construct: (target, args) => watch(new target(...args)) });
  }

  // Safari's prefixed API. Spotify's player uses it when WebKitMediaKeys exists.
  if (typeof WebKitMediaKeys === 'function') {
    window.WebKitMediaKeys = new Proxy(WebKitMediaKeys, {
      construct(target, args) {
        post('key-system', { keySystem: String(args[0]), api: 'webkit-prefixed', granted: true });
        return new target(...args);
      },
    });
  }

  // Now-playing metadata as Spotify publishes it to the Media Session (the
  // same data macOS shows in Control Center). Spotify's handlers are kept
  // so media keys keep working.
  // Set once Connect is wired up below.
  let refreshSoon = () => {};
  const reportNowPlaying = (value) => {
    const art = [...(value.artwork ?? [])]
      .sort((a, b) => parseInt(b.sizes) - parseInt(a.sizes))
      .map((image) => image.src);
    post('now-playing', { title: value.title, artist: value.artist, album: value.album, art });
    // A new song: read Spotify's state for its URI and details now rather
    // than at the next poll.
    refreshSoon();
  };
  const session = navigator.mediaSession;
  if (session) {
    const handlers = {};
    window.__spotifastMediaSession = (action, details) => {
      const handler = handlers[action];
      if (!handler) return post('control', { action, handled: false });
      handler({ action, ...details });
      post('control', { action, handled: true });
    };
    const setActionHandler = session.setActionHandler.bind(session);
    session.setActionHandler = (action, handler) => {
      handlers[action] = handler;
      return setActionHandler(action, handler);
    };
    const proto = Object.getPrototypeOf(session);
    const metadata = Object.getOwnPropertyDescriptor(proto, 'metadata');
    if (metadata?.set) {
      Object.defineProperty(session, 'metadata', {
        get() { return metadata.get.call(session); },
        set(value) {
          metadata.set.call(session, value);
          if (value) reportNowPlaying(value);
        },
      });
    }
  }

  // ---- Spotify Connect, from inside the page ------------------------------

  const pageFetch = window.fetch?.bind(window);
  const connect = { auth: null, device: null, prefix: null, host: null, license: '' };
  // The page's own requests to Spotify's API host carry its sign-in, and
  // its device registration names this player's Connect device id: the
  // web player makes a new random id each load (REA: randomizeId:!0) and
  // registers it with PUT .../connect-state/v1/devices/hobs_<id>
  // (REA: _register) or POST .../track-playback/v1/devices.
  const learn = (rawUrl, readHeader) => {
    try {
      const url = new URL(rawUrl, location.href);
      if (!/spclient(\.wg)?\.spotify\.com$/.test(url.hostname)) return;
      const authorization = readHeader('authorization');
      if (authorization) {
        // Not every request carries the client token; keep the last one.
        connect.auth = { authorization, clientToken: readHeader('client-token') ?? connect.auth?.clientToken ?? null };
        connect.host = url.host;
      }
      // "hobs_" plus the id, cut to 40 characters (REA: er=/^(...{1,40})/),
      // so this is the start of the id. The player's own state reports
      // (PUT .../track-playback/v1/devices/<id>/state) carry the whole id.
      const registered = url.pathname.match(/\/connect-state\/v1\/devices\/hobs_([A-Za-z0-9]+)$/);
      if (registered) connect.prefix = registered[1];
      const playing = url.pathname.match(/\/track-playback\/v1\/devices\/([A-Za-z0-9]+)\/state$/);
      if (playing) {
        connect.device = playing[1];
        connect.prefix = connect.prefix ?? playing[1];
      }
    } catch (_) {}
  };
  if (spotifyPage && pageFetch) {
    window.fetch = function (input, init) {
      const headers = new Headers(init?.headers ?? (typeof input === 'string' ? undefined : input.headers));
      learn(typeof input === 'string' ? input : input.url, (name) => headers.get(name));
      return pageFetch.apply(this, arguments);
    };
    const open = XMLHttpRequest.prototype.open;
    const setHeader = XMLHttpRequest.prototype.setRequestHeader;
    const send = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url, ...rest) {
      this.__spotifastUrl = String(url);
      this.__spotifastHeaders = {};
      return open.call(this, method, url, ...rest);
    };
    XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
      if (this.__spotifastHeaders) this.__spotifastHeaders[String(name).toLowerCase()] = String(value);
      return setHeader.call(this, name, value);
    };
    XMLHttpRequest.prototype.send = function (...args) {
      if (this.__spotifastUrl) learn(this.__spotifastUrl, (name) => this.__spotifastHeaders?.[name] ?? null);
      return send.apply(this, args);
    };
  }
  // Acknowledgements of this page's commands, as Spotify's dealer
  // connection delivers them: a cluster update naming the command's ack_id
  // (REA: _onClusterMessage -> trackCommandAcknowledged). Spotify's player
  // waits for one before trusting a command, up to 3400 ms (REA:
  // trackAcknowledgementRequired, CONNECTAPI_CLIENT_ACK_TIMEOUT).
  const ACK_TIMEOUT_MS = 3400;
  const acked = new Set();
  const ackWaiters = new Map();
  const acknowledge = (id) => {
    const waiter = ackWaiters.get(id);
    if (waiter) {
      ackWaiters.delete(id);
      waiter();
    } else {
      acked.add(id);
      setTimeout(() => acked.delete(id), ACK_TIMEOUT_MS);
    }
  };
  const acknowledged = (id) => new Promise((resolve) => {
    if (acked.delete(id)) return resolve(true);
    const timer = setTimeout(() => { ackWaiters.delete(id); resolve(false); }, ACK_TIMEOUT_MS);
    ackWaiters.set(id, () => { clearTimeout(timer); resolve(true); });
  });
  const PageSocket = spotifyPage ? window.WebSocket : null;
  if (PageSocket) {
    window.WebSocket = function (...args) {
      const socket = new PageSocket(...args);
      socket.addEventListener('message', (message) => {
        if (typeof message.data !== 'string' || !message.data.includes('ack_id')) return;
        try {
          const id = JSON.parse(message.data).payloads?.[0]?.ack_id;
          if (id) acknowledge(id);
        } catch (_) {}
      });
      return socket;
    };
    window.WebSocket.prototype = PageSocket.prototype;
    Object.setPrototypeOf(window.WebSocket, PageSocket);
  }
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const ready = async (ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (connect.auth && connect.prefix && !connect.device) await readState().catch(() => {});
      if (connect.auth && connect.device) return true;
      await sleep(300);
    }
    return false;
  };
  const headers = () => {
    const h = { authorization: connect.auth.authorization, 'content-type': 'application/json' };
    if (connect.auth.clientToken) h['client-token'] = connect.auth.clientToken;
    return h;
  };
  const base = () => `https://${connect.host}/connect-state/v1`;
  const commandId = () => (crypto.randomUUID?.() ?? `${Date.now()}${Math.random()}`).replace(/[^a-f0-9]/gi, '').slice(0, 32);

  // One Connect player command to this page's own device. A rejected token
  // (the page renews it about hourly) is retried once with the renewed one.
  // A 404 or 410 means Spotify does not know this device under a live
  // connection. Spotify's own player answers that by reading the cluster
  // again (REA: _awaitResponseWithAck -> _fetchCluster): if the device is
  // still listed the command goes once more, and if it is gone the page
  // reports "device-lost" so the helper can reload it and register again.
  const send = async (endpoint, fields = {}) => {
    const body = JSON.stringify({ command: { endpoint, ...fields, logging_params: { command_id: commandId() } } });
    let retriedLost = false;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const used = connect.auth.authorization;
      const response = await pageFetch(`${base()}/player/command/from/${connect.device}/to/${connect.device}`,
        { method: 'POST', headers: headers(), body });
      if (response.status === 401 && attempt === 0) {
        const end = Date.now() + 5000;
        while (connect.auth.authorization === used && Date.now() < end) await sleep(200);
        continue;
      }
      if ((response.status === 404 || response.status === 410) && !retriedLost) {
        retriedLost = true;
        if (await deviceListed()) continue;
        throw Object.assign(new Error('device lost'), { lost: true, status: response.status });
      }
      // Accepted: hold the next command until Spotify's player has taken
      // this one, as Spotify's own player does. A pause and play sent back
      // to back otherwise reach the player in the wrong order.
      const ackId = response.status === 200 ? (await response.json?.().catch(() => null))?.ack_id : null;
      if (ackId) await acknowledged(ackId);
      return response.status;
    }
    return 404;
  };

  // Whether Spotify's cluster still lists this page's device.
  const deviceListed = async () => {
    try {
      const response = await pageFetch(`${base()}/cluster`, { headers: headers() });
      if (!response.ok) return false;
      const cluster = await response.json();
      return !!cluster.devices?.[connect.device];
    } catch (_) {
      return false;
    }
  };

  // Spotify's clock: the Mac's clock plus an offset measured against
  // melody/v1/time, as the web player does (REA: _handleResponse adds half
  // the round trip). Refreshed every few minutes.
  let clockOffset = null;
  let clockAt = 0;
  const serverNow = async () => {
    if (clockOffset === null || Date.now() - clockAt > 300000) {
      try {
        const sent = Date.now();
        const response = await pageFetch(`https://${connect.host}/melody/v1/time`, { headers: headers() });
        const body = response.ok ? await response.json() : null;
        if (typeof body?.timestamp === 'number') {
          clockOffset = body.timestamp + Math.round((Date.now() - sent) / 2) - Date.now();
          clockAt = Date.now();
        }
      } catch (_) {}
    }
    return Date.now() + (clockOffset ?? 0);
  };

  // Spotify's playback state for this account, as the web player reads it.
  const held = (reasons) => (reasons ?? []).some((reason) => !['not_paused', 'already_set'].includes(reason));
  const restrictionsOf = (r) => ({
    skipNext: held(r.disallow_skipping_next_reasons),
    skipPrev: held(r.disallow_skipping_prev_reasons),
    seek: held(r.disallow_seeking_reasons),
    shuffle: held(r.disallow_toggling_shuffle_reasons),
    repeat: held(r.disallow_toggling_repeat_context_reasons) && held(r.disallow_toggling_repeat_track_reasons),
    queue: held(r.disallow_add_to_queue_reasons),
  });
  let lastState = '';
  const readState = async () => {
    if (!connect.auth || !connect.prefix) return;
    const response = await pageFetch(`${base()}/cluster`, { headers: headers() });
    if (!response.ok) return;
    const cluster = await response.json();
    connect.device = connect.device
      ?? Object.keys(cluster.devices ?? {}).find((id) => id.startsWith(connect.prefix)) ?? null;
    if (!connect.device) return;
    connect.license = cluster.devices?.[connect.device]?.license ?? connect.license;
    const player = cluster.player_state;
    if (!player || cluster.active_device_id !== connect.device) return;
    const options = player.options ?? {};
    const meta = player.track?.metadata ?? {};
    // Artists come as artist_name, artist_name:1, ... with matching uris.
    const artists = [];
    for (let i = 0; i < 20; i += 1) {
      const suffix = i ? `:${i}` : '';
      const name = meta[`artist_name${suffix}`];
      if (!name) break;
      artists.push({ name, uri: meta[`artist_uri${suffix}`] ?? null });
    }
    const state = {
      uri: player.track?.uri ?? null,
      contextUri: player.context_uri ?? null,
      title: meta.title ?? null,
      artists,
      album: meta.album_title ?? null,
      albumUri: meta.album_uri ?? null,
      art: meta.image_large_url ?? meta.image_url ?? null,
      artSmall: meta.image_small_url ?? meta.image_url ?? null,
      durationMs: Number(player.duration) || Number(meta.duration) || 0,
      shuffle: !!options.shuffling_context,
      repeat: options.repeating_track ? 'track' : options.repeating_context ? 'context' : 'off',
      paused: !!player.is_paused,
      // Spotify's live restrictions: a control is held while its reason
      // list is non-empty (REA: restrictions -> canSkipNext etc.). Reasons
      // that describe the current state, not a hold, are ignored.
      restrictions: restrictionsOf(player.restrictions ?? {}),
      // Spotify's upcoming songs, in order (REA: next_tracks feeds the web
      // player's queue view). Delimiters and hidden items are not songs.
      upcoming: (player.next_tracks ?? [])
        .filter((track) => track.uri?.startsWith('spotify:') && track.uri !== 'spotify:delimiter'
          && track.metadata?.hidden !== 'true')
        .slice(0, 50)
        .map((track) => track.uri),
    };
    const key = JSON.stringify(state);
    if (key !== lastState) {
      lastState = key;
      // Spotify's own player_state goes along unchanged, so Spotifast reads
      // it with the same tested types it checks against recorded responses
      // (src/connect_state.rs); the fields above are kept for older builds.
      post('state', { ...state, player: player });
    }
    // Position now: the state's position at its timestamp, moved on by the
    // time since, on Spotify's clock. The cluster's own timestamp is when
    // it last changed, not now; the page asks melody/v1/time for now and
    // keeps the offset (REA: _offsetPosition, getApproximate).
    const durationMs = Number(player.duration);
    const at = Number(player.position_as_of_timestamp);
    const elapsed = player.is_paused ? 0 : (await serverNow()) - Number(player.timestamp);
    if (durationMs > 0 && Number.isFinite(at)) {
      const positionMs = Math.max(0, Math.min(durationMs, at + (Number.isFinite(elapsed) ? elapsed * (player.playback_speed || 1) : 0)));
      post('progress', { positionMs: Math.round(positionMs), durationMs });
    }
  };
  const refresh = () => { for (const wait of [400, 1500, 4000]) setTimeout(() => readState().catch(() => {}), wait); };
  refreshSoon = refresh;

  const repeatOptions = (mode) => ({
    off: { repeating_context: false, repeating_track: false },
    context: { repeating_context: true, repeating_track: false },
    track: { repeating_context: true, repeating_track: true },
  }[mode]);

  // Spotifast's commands, as the web player's own Connect requests (REA:
  // play -> context + options.skip_to/seek_to/initially_paused and
  // player_options_override; seek_to {value}; set_shuffling_context
  // {value}; set_options {repeating_*}; add_to_queue {track}).
  const actions = {
    play: (args) => {
      const options = { license: connect.license };
      if (args.trackUri || Number.isInteger(args.trackIndex)) {
        options.skip_to = { track_uri: args.trackUri ?? undefined, track_index: args.trackIndex ?? undefined };
      }
      if (args.positionMs > 0) options.seek_to = args.positionMs;
      if (args.paused) options.initially_paused = true;
      const override = {};
      if (typeof args.shuffle === 'boolean') override.shuffling_context = args.shuffle;
      if (args.repeat) Object.assign(override, repeatOptions(args.repeat));
      if (Object.keys(override).length) options.player_options_override = override;
      // A list of songs plays as one page of tracks (REA: playPages), under
      // the playlist or album it came from when there is one.
      const uri = args.contextUri ?? 'spotify:internal:harmony-play-tracks';
      const context = { uri, url: `context://${uri}`, metadata: {} };
      if (args.tracks?.length) context.pages = [{ tracks: args.tracks.map((track) => ({ uri: track })) }];
      return send('play', {
        context,
        options,
        play_origin: { feature_identifier: 'harmony', feature_version: 'spotifast' },
      });
    },
    pause: () => send('pause'),
    resume: () => send('resume'),
    next: () => send('skip_next'),
    previous: () => send('skip_prev'),
    seek: (args) => send('seek_to', { value: Math.max(0, Math.round(args.positionMs ?? 0)) }),
    shuffle: (args) => send('set_shuffling_context', { value: !!args.on }),
    repeat: (args) => send('set_options', repeatOptions(args.mode) ?? repeatOptions('off')),
    queue: (args) => send('add_to_queue', { track: { uri: args.uri, metadata: { is_queued: 'true' }, provider: 'queue' } }),
    // Volume, as open.spotify.com sends it to a player: PUT
    // .../connect/volume/from/<id>/to/<id> with 0..65535 (REA:
    // _sendConnectCommand("volume")). Tested live: accepted and applied.
    volume: async (args) => {
      const volume = Math.max(0, Math.min(65535, Math.round(args.volume ?? 65535)));
      const response = await pageFetch(`${base()}/connect/volume/from/${connect.device}/to/${connect.device}`,
        { method: 'PUT', headers: headers(), body: JSON.stringify({ volume }) });
      return response.status;
    },
    // Radio, as open.spotify.com's "Go to radio": ask which playlist Spotify
    // builds for the seed (REA: inspiredby-mix/v2/seed_to_playlist), then
    // play it. Tested live on a Free account.
    radio: async (args) => {
      // The web player puts the URI in the path as is (REA: withPath
      // `/seed_to_playlist/${t}`); only Spotify URI characters are allowed.
      if (!/^spotify:(track|album|artist|playlist):[A-Za-z0-9]+$/.test(args.seed ?? '')) return 400;
      const seed = await pageFetch(
        `https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/${args.seed}?response-format=json`,
        { headers: headers() });
      const station = seed.ok ? (await seed.json())?.mediaItems?.[0]?.uri : null;
      if (!station) return seed.ok ? 404 : seed.status;
      post('radio', { seed: args.seed, station });
      return actions.play({ contextUri: station });
    },
  };
  // Set from Spotify's top bar by reportAccount below.
  let account = null;

  // Spotifast's reads for a Free account (REA: the web player's own
  // pathfinder queries). Spotify answers the official Web API for the
  // page's sign-in with "rate limit exceeded" from the first request, but
  // its own player's API answers. The query goes out from inside the page,
  // with the page's sign-in, which never leaves it; only the answer does.
  const PATHFINDER = 'https://api-partner.spotify.com/pathfinder/v2/query';
  // The account's saved songs and albums, newest first, as the web player
  // reads its library (REA: collection-platform). Only this read is let
  // through besides pathfinder queries.
  const COLLECTION = 'https://spclient.wg.spotify.com/collection/v2/paging';
  window.__spotifastQuery = async (id, query) => {
    // A signed-out page has an anonymous token too; a read with it answers
    // an empty profile. Wait for Spotify's own sign-in marker instead.
    reportAccount();
    const end = Date.now() + 30000;
    while ((!connect.auth || account === null) && Date.now() < end) {
      await sleep(200);
      reportAccount();
    }
    if (account === 'signed-out') return post('query', { id, status: 0, how: 'signed-out' });
    if (!connect.auth || account !== 'signed-in') return post('query', { id, status: 0, how: 'not-ready' });
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const used = connect.auth.authorization;
      try {
        const recent = query?.recentlyPlayed;
        const target = recent
          ? `https://spclient.wg.spotify.com/recently-played/v3/user/${encodeURIComponent(recent.username)}/recently-played?format=json&limit=${Number(recent.limit) || 50}&filter=default`
          : query?.collection ? COLLECTION : PATHFINDER;
        const body = query?.collection ? { ...query.collection } : query;
        const response = await pageFetch(target, recent
          ? { headers: { ...headers(), accept: 'application/json' } }
          : { method: 'POST', headers: { ...headers(), accept: 'application/json' }, body: JSON.stringify(body) });
        if (response.status === 401 && attempt === 0) {
          const renewed = Date.now() + 5000;
          while (connect.auth.authorization === used && Date.now() < renewed) await sleep(200);
          continue;
        }
        return post('query', { id, status: response.status, body: await response.text() });
      } catch (error) {
        return post('query', { id, status: 0, how: 'network', message: text(error) });
      }
    }
  };
  // Commands run one at a time, in the order pressed: each starts once the
  // one before it was acknowledged (or gave up), as Spotify's player sends.
  // A command stuck longer (a request that hangs, a wait for sign-in) holds
  // the next one only so long; then it goes, as it did before.
  const CHAIN_HOLD_MS = ACK_TIMEOUT_MS + 3000;
  let commandChain = Promise.resolve();
  window.__spotifastConnect = (action, args = {}) => {
    const turn = commandChain.then(() => runCommand(action, args));
    commandChain = Promise.race([turn.catch(() => {}), sleep(CHAIN_HOLD_MS)]);
    return turn;
  };
  const runCommand = async (action, args = {}) => {
    const run = actions[action];
    if (!run) return post('control', { action, handled: false, how: 'unknown-action' });
    // Signed out: the helper has shown Spotify's sign-in. Hold the command
    // until the page is signed in (up to ten minutes), then run it, so the
    // song the user picked starts once they are in.
    if (account === 'signed-out') {
      post('control', { action, handled: false, how: 'signing-in', waiting: true });
      const end = Date.now() + 600000;
      while (account !== 'signed-in' && Date.now() < end) await sleep(500);
    }
    if (!(await ready())) {
      return post('control', {
        action,
        handled: false,
        how: account === 'signed-out' ? 'not-signed-in' : connect.auth ? 'no-device' : 'not-signed-in',
      });
    }
    if (!connect.license) await readState().catch(() => {});
    try {
      const status = await run(args);
      post('control', { action, handled: status >= 200 && status < 300, status });
    } catch (error) {
      if (error?.lost) {
        post('control', { action, args, handled: false, how: 'device-lost', status: error.status });
      } else {
        post('control', { action, handled: false, how: 'network', message: text(error) });
      }
    }
    refresh();
  };

  // ---- Ads, read from Spotify's bar ---------------------------------------

  // REA (dwp-now-playing-bar, module 57591): while the playing item is an
  // ad, the bar renders "ad-controls" in place of "general-controls", in
  // both versions of its now-playing widget. Read-only.
  const byTestId = (id) => document.querySelector(`[data-testid="${id}"]`);
  const adLengthMs = () => {
    const max = Number(byTestId('playback-progressbar')?.querySelector('input[type="range"]')?.max);
    return Number.isFinite(max) && max > 0 ? max : null;
  };
  let adSince = null;
  const reportAd = () => {
    const now = !!byTestId('ad-controls');
    if (now && adSince === null) {
      adSince = Date.now();
      const title = byTestId('context-item-info-ad-title')?.textContent?.trim()
        ?? byTestId('context-item-info-title')?.textContent?.trim() ?? null;
      post('ad', { state: 'start', title, lengthMs: adLengthMs() });
    } else if (!now && adSince !== null) {
      post('ad', { state: 'end', playedMs: Date.now() - adSince });
      adSince = null;
      // The song may resume without Spotify publishing its metadata again;
      // report what is now playing so Spotifast leaves the ad.
      const metadata = navigator.mediaSession?.metadata;
      if (metadata) reportNowPlaying(metadata);
      lastState = '';
      refresh();
    }
  };

  // Whether this page is signed in to Spotify, from Spotify's own markers
  // (REA, web-player top bar: data-testid "login-button" while signed out,
  // "user-widget-link" once signed in). Reported when it settles and each
  // time it changes, so the helper can show its window for sign-in and
  // hide it again by itself.
  const reportAccount = () => {
    const now = byTestId('user-widget-link') ? 'signed-in'
      : byTestId('login-button') ? 'signed-out' : null;
    if (now && now !== account) {
      account = now;
      post('account', { state: now });
    }
  };

  if (spotifyPage) {
    setInterval(reportAd, 500);
    setInterval(reportAccount, 1000);
    // Spotify's state while something plays; commands also read it at once.
    setInterval(() => { if (document.querySelector('audio, video') || lastState) readState().catch(() => {}); }, 2000);
  }

  const request = navigator.requestMediaKeySystemAccess?.bind(navigator);
  if (request) {
    navigator.requestMediaKeySystemAccess = (keySystem, configs) =>
      request(keySystem, configs).then(
        (access) => { post('key-system', { keySystem, granted: true }); return access; },
        (error) => { post('key-system', { keySystem, granted: false, error: error?.name }); throw error; });
  }
})();
