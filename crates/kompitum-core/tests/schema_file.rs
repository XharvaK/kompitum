//! The committed schema is the code's, on Windows too. `m0` holds this check for Unix but
//! compiles to nothing here, so `--test m0` would pass without looking at the file.
#![cfg(windows)]

#[test]
fn the_committed_schema_file_matches_the_code() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/kom.openrpc.json");
    let generated = serde_json::to_string_pretty(&kompitum_core::api::openrpc()).unwrap() + "\n";
    if std::env::var_os("KOMPITUM_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "schema/kom.openrpc.json is stale: review the API change, then KOMPITUM_UPDATE_SCHEMA=1 cargo test --test schema_file"
    );
}
