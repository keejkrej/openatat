//! `openatat-ui` — on-demand gpui-ce Settings / History / Studio.
//!
//! Spawned by `openatatd --settings` / `--history` / `--studio` or run directly.
//! Quits when the last window closes. This is **not** the @@ overlay.

fn main() {
    let cli = openatat_ui::Cli::parse(std::env::args().skip(1));
    if cli.help {
        print!("{}", openatat_ui::HELP);
        return;
    }

    #[cfg(feature = "gpui")]
    {
        if let Err(e) = openatat_ui::ui::run(cli) {
            eprintln!("openatat-ui: {e}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(not(feature = "gpui"))]
    {
        let _ = cli;
        eprintln!(
            "openatat-ui: built without the `gpui` feature (keeps cargo test GPU-free).\n\
             \n\
             Rebuild the window:\n\
               cargo build -p openatat-ui --features gpui\n\
               cargo run -p openatat-ui --features gpui -- --settings\n\
               cargo run -p openatat-ui --features gpui -- --studio --image some.png\n\
             \n\
             Linux compile needs: libxkbcommon-dev libwayland-dev libvulkan-dev\n\
             (and usually libfontconfig-dev). See README.md.\n\
             Overlay / Orb / trigger stay in openatatd — never here."
        );
        std::process::exit(2);
    }
}
