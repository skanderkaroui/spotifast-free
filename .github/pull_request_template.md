## Why

What user problem does this solve, and why does it belong in this fork? If it
is not about Free playback, send it to upstream Spotifast instead.

## What changed

Describe the change. Link the issue, if there is one.

## Verification

List the commands and platforms you actually tested. For UI changes, drag
before/after screenshots or a recording into this description so they attach
here; do not commit them.

- [ ] I read `CONTRIBUTING.md` and kept this pull request to one concern.
- [ ] I added or updated tests for behaviour that can regress.
- [ ] I ran formatting, Clippy, and the relevant tests locally.
- [ ] I updated documentation for user-visible behaviour, settings, files, or network access.
- [ ] Screenshots, captures, and recordings are attached here, not committed.
- [ ] This adds no setting, or the maintainer approved it in the linked issue (see "Settings" in `CONTRIBUTING.md`).
- [ ] Dependencies come from crates.io or `github.com/crmne` repositories, and the maintainer approved any added or changed git source.
- [ ] I understand and take responsibility for every submitted line, including AI-assisted code.
- [ ] Free playback: Premium behaviour is unchanged, and `src/app/free_matrix_tests.rs` passes for both plans.
- [ ] This does not skip, block, mute, shorten or hide ads, or work around any Spotify Free restriction (see "Ads stay" in `CONTRIBUTING.md`).
- [ ] Free playback: the helper still leaves Spotify's player and DRM untouched.
- [ ] Recordings went through `scrub-recordings.mjs`; nothing personal (songs, playlists, ids, paths, sign-in) is committed.
