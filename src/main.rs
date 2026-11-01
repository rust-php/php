use std::{env, fs, io::Write, process::ExitCode};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: php <file.php>");
        return ExitCode::from(2);
    };
    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open input file: {path}: {e}");
            return ExitCode::from(1);
        }
    };
    match php::run_source(&src) {
        Ok(out) => {
            let _ = std::io::stdout().write_all(out.as_bytes());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("PHP Fatal error:  {e}");
            ExitCode::from(255)
        }
    }
}
