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
    // global-hotkey and the tray icon both require the AppKit event loop on
    // the main thread.  Tokio runs on a named background thread instead.
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
    run_macos_main()?;

    #[cfg(not(target_os = "macos"))]
    std::thread::park();

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_macos_main() -> anyhow::Result<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    // Give tokio a moment to start and register the hotkey.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Accessory: no Dock icon, doesn't steal focus.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Sends applicationDidFinishLaunching; required before creating NSStatusItem.
    app.finishLaunching();

    let ptt_key = callout::Config::load()
        .map(|c| c.hotkey.ptt)
        .unwrap_or_else(|_| "Alt+K".into());

    let tray = callout::tray::build(&ptt_key)?;

    // NSEvent global monitors (used by global-hotkey) only fire when the
    // AppKit event queue is drained via nextEventMatchingMask:…  CFRunLoopRun
    // alone is not enough — we must call this method the way app.run() does.
    loop {
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask(u64::MAX),
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.05)),
                NSDefaultRunLoopMode,
                true,
            )
        };
        if let Some(event) = event {
            app.sendEvent(&event);
        }

        if callout::tray::poll(&tray) {
            std::process::exit(0);
        }
    }
}

fn ptt_test() -> anyhow::Result<()> {
    let config = callout::Config::load()?;
    println!(
        "PTT key: \"{}\" — check Input Monitoring in System Settings if it doesn't respond.",
        config.hotkey.ptt
    );
    Ok(())
}
