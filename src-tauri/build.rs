#[path = "build/easytier.rs"]
mod easytier;

fn main() {
    easytier::embed();
    tauri_build::build();
}
