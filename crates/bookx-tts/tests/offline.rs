//! Offline mode stops downloads before any connection is made. Own test binary: the switch is
//! process-wide.

use bookx_tts::download::{download_file, is_offline, set_offline};

#[test]
fn offline_mode_refuses_a_download() {
    let dir = std::env::temp_dir().join(format!("bookx-tts-offline-{}", uuid::Uuid::new_v4()));
    let dest = dir.join("voice.onnx");
    set_offline(true);
    assert!(is_offline());
    let cancel = bookx_tts::synth::Cancel::new();
    // port 9 would refuse the connection; the error must be ours, so nothing was tried
    let err = download_file("http://127.0.0.1:9/x", &dest, "", 10, &cancel, &mut |_| {})
        .unwrap_err()
        .to_string();
    set_offline(false);
    assert!(err.contains("Offline mode"), "{err}");
    assert!(!dest.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
