fn main() {
    if let Err(error) = claude_relink::run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
