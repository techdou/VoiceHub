#[path = "../vendor/sayit/native/build.rs"]
mod runtime;
fn main() {
    runtime::stage_transcribe_runtime_libs();
    tauri_build::build()
}
