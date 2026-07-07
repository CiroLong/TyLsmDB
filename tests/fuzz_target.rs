use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn fuzz_model_binary_accepts_arbitrary_operation_bytes() {
    let binary = std::env::var("CARGO_BIN_EXE_fuzz_model").expect("fuzz_model binary path");
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fuzz_model binary");

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(&[
            0, 1, 10, 1, 2, 11, 3, 1, 0, 5, 0, 0, 6, 0, 0, 2, 1, 0, 4, 0, 0,
        ])
        .expect("write fuzz input");

    let output = child.wait_with_output().expect("wait fuzz_model");
    assert!(
        output.status.success(),
        "fuzz_model failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
