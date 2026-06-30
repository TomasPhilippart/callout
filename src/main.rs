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
    //
    // We send AppState through a channel so the main thread can read live
    // agent and recording state for tray menu updates.
    let (state_tx, state_rx) = std::sync::mpsc::sync_channel::<callout::AppState>(1);

    std::thread::Builder::new()
        .name("callout-tokio".into())
        .spawn(move || {
            if let Err(e) = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(callout::run_with(state_tx))
            {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        })?;

    #[cfg(target_os = "macos")]
    run_macos_main(state_rx)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = state_rx;
        std::thread::park();
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_macos_main(state_rx: std::sync::mpsc::Receiver<callout::AppState>) -> anyhow::Result<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    // Give tokio a moment to start and register the global hotkey.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Accessory: no Dock icon, doesn't steal focus.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Required before creating NSStatusItem (tray icon).
    app.finishLaunching();

    let ptt_key = callout::Config::load()
        .map(|c| c.hotkey.ptt)
        .unwrap_or_else(|_| "Alt+K".into());

    let tray = callout::tray::build(&ptt_key)?;

    // Wait for AppState (sent after Whisper loads, usually ~2 s)
    let app_state = state_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .ok();

    // NSEvent global monitors (used by global-hotkey) only fire when AppKit's
    // event queue is drained via nextEventMatchingMask:… — CFRunLoopRunInMode
    // alone does not call this, so hotkeys would silently stop working.
    let mut tick: u32 = 0;
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

        if callout::tray::poll(&tray, app_state.as_ref()) {
            std::process::exit(0);
        }

        tick = tick.wrapping_add(1);
        if let Some(state) = &app_state {
            // Icon/tooltip: every tick (~50 ms) for instant feedback on PTT press.
            callout::tray::update_icon(&tray, state);
            // Menu (needs locks): every 10 ticks (~500 ms) is plenty.
            if tick.is_multiple_of(10) {
                callout::tray::update_menu(&tray, state);
            }
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
