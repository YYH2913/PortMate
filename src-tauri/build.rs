#[path = "build/easytier.rs"]
mod easytier;
#[path = "build/shell_elevation.rs"]
mod shell_elevation;

fn main() {
    easytier::embed();
    shell_elevation::embed();
    tauri_build::build();
}
