fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env = bookx_cli::Env::from_process();
    // not locked: `mcp` writes protocol messages from more than one thread
    let code = bookx_cli::run(&args, &env, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
