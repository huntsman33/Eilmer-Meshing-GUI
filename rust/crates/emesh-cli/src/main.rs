//! emesh-cli: the command-line tools without the app (for scripts and CI).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(|s| s.as_str()), Some("--version" | "-V")) {
        println!("emesh-cli {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match emesh_cli::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("emesh: {e}");
            ExitCode::FAILURE
        }
    }
}
