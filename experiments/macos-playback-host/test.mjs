import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

const script = await readFile(new URL('MediaCapabilities.js', import.meta.url), 'utf8');
async function probe(navigator) {
  const output = await runInNewContext(`(async () => { ${script} })()`, {
    navigator, location: { origin: 'https://open.spotify.com' },
    MediaSource: class {}, setTimeout, clearTimeout,
  });
  return JSON.parse(output);
}

const absent = await probe({});
assert.equal(absent.eme, false);
assert.deepEqual(absent.keySystems, {});

const requests = [];
const supported = await probe({
  requestMediaKeySystemAccess: async (key, configs) => {
    requests.push({ key, configs });
    if (key !== 'com.widevine.alpha') throw { name: 'NotSupportedError' };
    return { createMediaKeys() { throw new Error('Probe must not create keys'); } };
  },
});
assert.equal(supported.keySystems['com.widevine.alpha'][0].accepted, true);
assert.equal(supported.keySystems['com.apple.fps'][0].error, 'NotSupportedError');
assert.equal(requests.length, 5);
assert.ok(requests.every(({ configs }) => configs[0].sessionTypes[0] === 'temporary'));

const failed = await probe({
  requestMediaKeySystemAccess: async () => { throw { name: 'credential-in-error', message: 'private' }; },
});
assert.ok(!JSON.stringify(failed).includes('credential-in-error'));
assert.ok(!JSON.stringify(failed).includes('private'));
assert.equal(failed.keySystems['com.widevine.alpha'][0].error, 'OtherError');
console.log('Media capability probe checks passed. No keys or sessions were created.');
