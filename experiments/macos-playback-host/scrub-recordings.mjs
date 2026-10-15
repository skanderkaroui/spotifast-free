// Makes recorded open.spotify.com responses safe to commit as test data.
//
//   node experiments/macos-playback-host/scrub-recordings.mjs <in-dir> <out-dir>
//
// Every .json file in <in-dir> is processed together, so the same song,
// artist or playlist gets the same placeholder in every file:
// - Spotify item URIs become placeholders of the same kind and length
//   (spotify:track:track00000000000000001), also inside longer strings.
// - Song, artist and album names become "Song 1", "Artist 1", "Album 1".
// - Artwork URLs, row uids and highlight ids are replaced or dropped.
// - next_tracks keeps the first three visible rows plus one delimiter and
//   one hidden row, in their original order; prev_tracks keeps two.
// The structure, flags, restrictions, options and timings stay as recorded:
// those are what the parsers are tested against. Device and session ids
// must already be replaced (the recordings carry `own_device`).
import { readdir, readFile, writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const [inDir, outDir] = process.argv.slice(2);
if (!inDir || !outDir) {
  console.error('usage: scrub-recordings.mjs <in-dir> <out-dir>');
  process.exit(2);
}

const counters = new Map();
const mapped = new Map();
const placeholder = (space, original, make) => {
  const key = `${space}\u0000${original}`;
  if (!mapped.has(key)) {
    const n = (counters.get(space) ?? 0) + 1;
    counters.set(space, n);
    mapped.set(key, make(n));
  }
  return mapped.get(key);
};
const itemUri = /spotify:(track|artist|album|playlist|episode|show):([A-Za-z0-9]{22})/g;
const scrubString = (s) =>
  s.replace(itemUri, (whole, kind) =>
    placeholder(kind, whole, (n) => `spotify:${kind}:${kind}${String(n).padStart(22 - kind.length, '0')}`));

const nameKeys = [
  [/^title$/, 'Song'],
  [/^artist_name(:\d+)?$/, 'Artist'],
  [/^album_title$/, 'Album'],
];
const scrubMetadata = (metadata) => {
  const out = {};
  for (const [key, value] of Object.entries(metadata)) {
    if (key === 'highlight_id') continue;
    const name = nameKeys.find(([pattern]) => pattern.test(key));
    if (name && typeof value === 'string') {
      out[key] = placeholder(name[1], value, (n) => `${name[1]} ${n}`);
    } else if (/^image(_small|_large)?_url$/.test(key)) {
      out[key] = placeholder('image', value, (n) => `https://i.scdn.co/image/image${String(n).padStart(35, '0')}`);
    } else {
      out[key] = typeof value === 'string' ? scrubString(value) : value;
    }
  }
  return out;
};

const scrub = (value, key) => {
  if (Array.isArray(value)) return value.map((item) => scrub(item));
  if (value && typeof value === 'object') {
    const out = {};
    for (const [k, v] of Object.entries(value)) {
      if (k === 'metadata' && v && typeof v === 'object' && !Array.isArray(v)) out[k] = scrubMetadata(v);
      else out[k] = scrub(v, k);
    }
    return out;
  }
  if (typeof value !== 'string') return value;
  if (key === 'uid' && !value.startsWith('delimiter')) {
    return placeholder('uid', value, (n) => `uid${String(n).padStart(17, '0')}`);
  }
  return scrubString(value);
};

const isDelimiter = (row) => row.uri === 'spotify:delimiter';
const isHidden = (row) => row.metadata?.hidden === 'true';
const trimNext = (rows) => {
  const keep = new Set();
  let visible = 0;
  rows.forEach((row, i) => {
    if (!isDelimiter(row) && !isHidden(row) && visible < 3) {
      keep.add(i);
      visible += 1;
    }
  });
  const delimiter = rows.findIndex(isDelimiter);
  if (delimiter >= 0) keep.add(delimiter);
  const hidden = rows.findIndex((row) => !isDelimiter(row) && isHidden(row));
  if (hidden >= 0) keep.add(hidden);
  return rows.filter((_, i) => keep.has(i));
};

const files = (await readdir(inDir)).filter((name) => name.endsWith('.json')).sort();
const trimmed = [];
for (const name of files) {
  const recording = JSON.parse(await readFile(join(inDir, name), 'utf8'));
  const state = recording.cluster?.player_state;
  if (state) {
    if (Array.isArray(state.next_tracks)) state.next_tracks = trimNext(state.next_tracks);
    if (Array.isArray(state.prev_tracks)) state.prev_tracks = state.prev_tracks.slice(-2);
  }
  trimmed.push([name, recording]);
}
await mkdir(outDir, { recursive: true });
for (const [name, recording] of trimmed) {
  const { _source: source, ...rest } = recording;
  const out = {
    _source: `${scrubString(String(source ?? 'open.spotify.com').split(',')[0])}, Free account; trimmed by scrub-recordings.mjs, ids, songs and playlists replaced`,
    ...scrub(rest),
  };
  await writeFile(join(outDir, name), `${JSON.stringify(out, null, 1)}\n`);
}
for (const [key, value] of mapped) {
  const [, original] = key.split('\u0000');
  if (original.startsWith('spotify:')) console.log(`${original} -> ${value}`);
}
