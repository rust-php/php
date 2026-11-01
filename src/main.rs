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
    // Run on a big stack so deep PHP recursion does not overflow the Rust stack.
    let worker = std::thread::Builder::new().stack_size(1 << 30).spawn(move || php::execute(&src));
    let out = match worker.map(|h| h.join()) {
        Ok(Ok(o)) => o,
        _ => {
            eprintln!("internal error: interpreter thread failed");
            return ExitCode::from(70);
        }
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.out.as_bytes());
    if let Some(m) = &out.fatal {
        let _ = write!(stdout, "\nFatal error: {m}\n");
        eprintln!("PHP Fatal error:  {m}");
    }
    let _ = stdout.flush();
    ExitCode::from(out.exit_code as u8)
}
