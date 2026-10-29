use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn third_party_notices_are_current_and_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| {
        fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n")
    };
    let notices = read("THIRD-PARTY-NOTICES");
    let inventory: serde_json::Value =
        serde_json::from_str(&read("licenses/inventory.json")).unwrap();
    let mut inputs = BTreeSet::new();
    for path in inventory["inputs"].as_array().unwrap() {
        inputs.insert(path.as_str().unwrap());
    }
    assert!(inputs.contains("Cargo.lock"));
    for component in inventory["components"].as_array().unwrap() {
        for path in component["files"].as_array().unwrap() {
            let path = path.as_str().unwrap();
            inputs.insert(path);
            assert!(
                notices.contains(read(path).trim()),
                "{path} missing from THIRD-PARTY-NOTICES; run just notices"
            );
        }
    }
    for path in inputs {
        let hash = format!("{:x}", Sha256::digest(read(path).as_bytes()));
        assert!(
            notices
                .lines()
                .any(|line| line == format!("Input-SHA256: {hash} {path}")),
            "{path} changed; review licenses and run just notices"
        );
    }
    assert!(notices.contains("Cargo dependencies"));
    assert!(notices.contains("quickjs-jit-sys"));
    assert!(notices.contains("gpui-pre"));
}
