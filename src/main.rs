use audio_file_synthesis_mcp::mcp::server::run_stdio_server;

fn main() {
    let work_dir = parse_work_dir();
    if let Err(error) = run_stdio_server(work_dir) {
        eprintln!("audio_file_synthesis_mcp failed: {error}");
        std::process::exit(1);
    }
}

fn parse_work_dir() -> std::path::PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--work-dir" {
            if let Some(value) = args.next() {
                return std::path::PathBuf::from(value);
            }
        } else if let Some(value) = arg.strip_prefix("--work-dir=") {
            return std::path::PathBuf::from(value);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}
