#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    let mut args = std::env::args_os().skip(1);
    if let Some(arg) = args.next() {
        if arg == "--configure-network" {
            let exit_code = if args.next().is_none() {
                pc_remote_desktop_lib::configure_network_for_installer()
            } else {
                3
            };
            std::process::exit(exit_code);
        }
    }
    pc_remote_desktop_lib::run()
}
