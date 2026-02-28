use audio_file_synthesis_mcp::mcp::server::run_stdio_server;

fn main() {
    if let Err(error) = run_stdio_server() {
        eprintln!("audio_file_synthesis_mcp failed: {error}");
        std::process::exit(1);
    }
}
