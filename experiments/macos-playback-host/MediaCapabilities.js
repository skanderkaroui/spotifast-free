// Capability negotiation only. No media sessions, license requests or credentials.
const result = {
  origin: location.origin,
  eme: typeof navigator.requestMediaKeySystemAccess === 'function',
  mediaSource: typeof MediaSource !== 'undefined',
  legacyMediaKeys: typeof WebKitMediaKeys !== 'undefined',
  keySystems: {},
};

if (result.eme) {
  const audio = [{ contentType: 'audio/mp4; codecs="mp4a.40.2"' }];
  const profiles = {
    'com.widevine.alpha': [['cenc'], ['webm']],
    'com.apple.fps': [['sinf'], ['skd']],
    'com.apple.fps.1_0': [['sinf'], ['skd']],
  };
  for (const [keySystem, initDataTypes] of Object.entries(profiles)) {
    const attempts = [];
    for (const types of initDataTypes) {
      let timer;
      try {
        await Promise.race([
          navigator.requestMediaKeySystemAccess(keySystem, [{
            initDataTypes: types,
            audioCapabilities: audio,
            distinctiveIdentifier: 'optional',
            persistentState: 'optional',
            sessionTypes: ['temporary'],
          }]),
          new Promise((_, reject) => {
            timer = setTimeout(() => reject({ name: 'TimeoutError' }), 3000);
          }),
        ]);
        attempts.push({ initDataTypes: types, accepted: true });
        break;
      } catch (error) {
        const names = ['NotSupportedError', 'SecurityError', 'InvalidStateError', 'TimeoutError'];
        attempts.push({
          initDataTypes: types,
          accepted: false,
          error: names.includes(error?.name) ? error.name : 'OtherError',
        });
      } finally {
        clearTimeout(timer);
      }
    }
    result.keySystems[keySystem] = attempts;
  }
}
return JSON.stringify(result);
