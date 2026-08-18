// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--daemon` runs the headless process supervisor instead of the GUI.
    // Same binary on purpose: one .app bundle to sign, notarize and ship,
    // and the daemon can never drift out of sync with the app that spawned
    // it.
    if std::env::args().any(|a| a == "--daemon") {
        if let Err(e) = easy_term_lib::run_daemon() {
            eprintln!("easy-term daemon: {e}");
            std::process::exit(1);
        }
        return;
    }

    easy_term_lib::run()
}
