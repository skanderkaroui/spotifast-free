// Drives the Linux helper over its protocol, as Spotifast does: waits for
// sign-in, sends a play, checks the media plays past 0:35, pauses, quits.
const { spawn } = require('child_process');
const electron = process.argv[2];
const helper = spawn(electron, ['--no-sandbox', __dirname, '--headless'], {
  env: { ...process.env, SPOTIFAST_HELPER_PROFILE: process.env.SPOTIFAST_HELPER_PROFILE },
  stdio: ['pipe', 'pipe', 'ignore'],
});
const send = (o) => helper.stdin.write(JSON.stringify(o) + '\n');
let buffer = '';
let maxMs = 0;
let signedIn = false;
let played = false;
helper.stdout.on('data', (d) => {
  buffer += d;
  let i;
  while ((i = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, i); buffer = buffer.slice(i + 1);
    let e; try { e = JSON.parse(line); } catch { continue; }
    if (e.event === 'account' || e.event === 'control' || e.event === 'probe-error' ||
        (e.event === 'media' && ['playing', 'pause', 'error'].includes(e.type))) console.log(line.slice(0, 220));
    if (e.event === 'media' && e.positionMs > maxMs) maxMs = e.positionMs;
    if (e.event === 'query') console.log('QUERY', e.id, e.status ?? '', String(line).length);
    if (e.event === 'account' && e.state === 'signed-in' && !signedIn) {
      signedIn = true;
      send({ cmd: 'query', id: 1, query: { operationName: 'profileAttributes', variables: {}, extensions: { persistedQuery: { version: 1, sha256Hash: '53bcb064f6cd18c23f752bc324a791194d20df612d8e1239c735144ab0399ced' } } } });
      send({ cmd: 'connect', action: 'play', args: { tracks: ['spotify:track:4cOdK2wGLETKBW3PvgPWqT'] } });
      played = true;
      setTimeout(() => send({ cmd: 'connect', action: 'pause', args: {} }), 45000);
      setTimeout(() => send({ cmd: 'quit' }), 50000);
    }
  }
});
helper.on('exit', (code) => {
  console.log(`RESULT signedIn=${signedIn} played=${played} maxPositionMs=${maxMs} exit=${code}`);
  process.exit(signedIn && maxMs > 35000 ? 0 : 1);
});
setTimeout(() => { console.log('TIMEOUT'); helper.kill(); }, 110000);
