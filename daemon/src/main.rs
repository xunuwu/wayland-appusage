use tracing::{error, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;

mod app;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .without_time()
        .init();

    let wayland_connection = wayland_client::Connection::connect_to_env()
        .expect("Failed to connect to wayland server");

    let mut queue = {
        let display = wayland_connection.display();

        let queue = wayland_connection.new_event_queue();
        let queue_handle = queue.handle();

        display.get_registry(&queue_handle, ());

        queue
    };

    let mut state = app::AppState::new().expect("Initialization failed");

    if let Err(e) = queue.roundtrip(&mut state) {
        error!("Roundtrip failed: {e}");
    }

    if state.toplevel_manager.is_none() {
        error!("Failed to get toplevel manager, does you compositor implement wlr-foreign-toplevel-management-unstable?");
        return;
    }

    if let Some(ref idle_notifier) = state.idle_notifier {
        idle_notifier.get_idle_notification(30_000, &state.seats[0], &queue.handle(), ());
    } else {
        error!("Failed to get idle notifier, does you compositor implement ext-idle-notify?");
        return;
    }

    loop {
        queue
            .blocking_dispatch(&mut state)
            .expect("Wayland dispatch failed");
    }
}
