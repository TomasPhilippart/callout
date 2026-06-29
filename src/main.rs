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
    // global-hotkey and the tray icon both require the main thread to run the
    // AppKit event loop.  Tokio runs on a named background thread instead.
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
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    // Give tokio a moment to register the global hotkey before we start
    // pumping the event loop.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Accessory: no Dock icon, doesn't take focus.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // finishLaunching sends applicationDidFinishLaunching and activates the
    // app; required before creating an NSStatusItem (tray icon).
    app.finishLaunching();

    let ptt_key = callout::Config::load()
        .map(|c| c.hotkey.ptt)
        .unwrap_or_else(|_| "Alt+K".into());

    let tray = callout::tray::build(&ptt_key)?;

    // Manual event loop: pump AppKit for 50 ms at a time, then poll channels.
    // This lets both global-hotkey and tray-icon events be delivered while
    // keeping the main thread available for Objective-C callbacks.
    loop {
        unsafe { pump_run_loop(0.05) };

        if callout::tray::poll(&tray) {
            std::process::exit(0);
        }
    }
}

/// Run the CoreFoundation run loop for `seconds`, then return.
/// Processes all pending AppKit / global-hotkey / NSMenu events.
#[cfg(target_os = "macos")]
unsafe fn pump_run_loop(seconds: f64) {
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const core::ffi::c_void;
        fn CFRunLoopRunInMode(
            mode: *const core::ffi::c_void,
            seconds: f64,
            return_after_source_handled: u8,
        ) -> i32;
    }
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 0);
}

fn ptt_test() -> anyhow::Result<()> {
    let config = callout::Config::load()?;
    println!(
        "PTT key: \"{}\" — check Input Monitoring in System Settings if it doesn't respond.",
        config.hotkey.ptt
    );
    Ok(())
}
