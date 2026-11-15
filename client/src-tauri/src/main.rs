#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--mcp-stdio") {
        std::process::exit(cloud_deck::mcp_stdio(&args[1..]));
    }
    cloud_deck::run()
}
