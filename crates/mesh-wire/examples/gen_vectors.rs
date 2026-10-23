//! Writes the wire golden vectors (DESIGN §4.10) into the directory given as the only
//! argument, one pretty-printed JSON file per group. Driven by `cargo xtask vectors`,
//! which compares the output with `test-vectors/wire/` in `--check` mode.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: gen_vectors <output-dir>");
        return ExitCode::from(2);
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("gen_vectors: cannot create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let files = match mesh_wire::vectors::generate() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gen_vectors: generation failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    for f in &files {
        let path = dir.join(&f.file_name);
        if let Err(e) = std::fs::write(&path, f.render()) {
            eprintln!("gen_vectors: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    println!(
        "gen_vectors: wrote {} files to {}",
        files.len(),
        dir.display()
    );
    ExitCode::SUCCESS
}
