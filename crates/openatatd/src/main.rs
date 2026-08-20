fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = openatatd::Cli::parse(&args);
    if let Err(e) = openatatd::run(cli) {
        eprintln!("openatatd: {e}");
        std::process::exit(1);
    }
}
