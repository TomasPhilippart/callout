use callout::cli::{Cli, Command};
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Serve) => run_serve(),
        Some(Command::Voices { cmd }) => callout::voices::run(cmd),
        Some(Command::Model { cmd }) => callout::model::run(cmd),
        Some(Command::PttTest) => ptt_test(),
    }
}

fn run_serve() -> anyhow::Result<()> {
    // global-hotkey delivers keyboard events via NSApplication on macOS.
    // NSApplication.run() must own the main thread, so tokio runs on a
    // named background thread instead.
    std::thread::Builder::new()
        .name("callout-tokio".into())
        .spawn(|| {
            if let Err(e) = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(callout::run())
            {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        })?;

    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

        // Give the tokio thread a moment to start and register the hotkey
        // before the event loop begins consuming events.
        std::thread::sleep(std::time::Duration::from_millis(200));

        let mtm = MainThreadMarker::new().expect("must run on main thread");
        let app = NSApplication::sharedApplication(mtm);
        // Accessory: no Dock icon, no menu bar, doesn't steal focus
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        // Blocks forever, pumping AppKit events — required for global-hotkey
        app.run();
    }

    #[cfg(not(target_os = "macos"))]
    std::thread::park();

    Ok(())
}

fn ptt_test() -> anyhow::Result<()> {
    let config = callout::Config::load()?;
    println!(
        "PTT key: \"{}\" — use global-hotkey; check Input Monitoring if it doesn't respond.",
        config.hotkey.ptt
    );
    Ok(())
}
