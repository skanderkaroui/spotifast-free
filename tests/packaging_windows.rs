//! Spotifast Free's Windows installers carry their own identity. Windows
//! tells an update from a new program by these IDs, so with upstream
//! Spotifast's, installing one would replace or uninstall the other.
use std::fs;
use std::path::PathBuf;

fn packaging(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("packaging/windows")
        .join(file);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .replace("\r\n", "\n")
}

/// Upstream Spotifast's installer IDs and name, which this fork must not use.
const UPSTREAM: &[&str] = &[
    "FCED1EA0-EBF5-4C32-BA3B-A3AD724BACC3",
    "3DFFED43-88FE-4A31-9461-50BE03094316",
    "8EC6378E-B3EB-41F9-873B-01AE4466E5EE",
];

#[test]
fn the_installers_do_not_reuse_upstream_spotifasts_identity() {
    for file in ["spotifast.wxs", "spotifast.iss"] {
        let text = packaging(file);
        for id in UPSTREAM {
            assert!(!text.contains(id), "{file} still carries upstream's {id}");
        }
    }
    // Registry keys, the install folder and the link class are named after
    // the identity, so it is one word, like the app's own folders.
    assert!(packaging("spotifast.wxs").contains(r#"<?define Identity = "SpotifastFree" ?>"#));
    let inno = packaging("spotifast.iss");
    assert!(inno.contains(r#"#define AppName "Spotifast Free""#));
    assert!(inno.contains(r#"#define AppIdentity "SpotifastFree""#));
}

/// The two installers name the same program: the MSI removes an Inno Setup
/// install by its AppId, so both must agree on it.
#[test]
fn the_msi_and_inno_setup_agree_on_the_app_id() {
    let wxs = packaging("spotifast.wxs");
    let inno = packaging("spotifast.iss");
    let msi_id = wxs
        .split(r#"<?define InnoAppId = ""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the MSI names Inno Setup's AppId");
    let inno_id = inno
        .lines()
        .find_map(|line| line.strip_prefix("AppId={"))
        .expect("Inno Setup's AppId");
    assert_eq!(msi_id, inno_id, "the MSI's InnoAppId is Inno Setup's AppId");
}
