use std::fs;
use std::path::PathBuf;

use super::{read_current, set_current};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn current_points_to_manifest() {
    let path = fresh_dir("current_points_to_manifest");

    set_current(&path, "MANIFEST-000001").expect("set current");

    assert_eq!(
        fs::read_to_string(path.join("CURRENT")).expect("read current file"),
        "MANIFEST-000001\n"
    );
    assert_eq!(
        read_current(&path).expect("read current helper"),
        "MANIFEST-000001"
    );
}
