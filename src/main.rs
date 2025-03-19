// minutes per app:
// select app_name, sum(duration) / (60 * 1000) as total_usage from app_usage group by app_name order by total_usage desc

// TODO exit when ran on a system that doesnt support the neccessary protocols
// TODO program for visualizing this data!! maybe using plotters or some js library with tauri?

use std::{
    collections::HashMap,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::params;
use wayland_client::{
    Dispatch, event_created_child,
    protocol::{wl_registry, wl_seat::WlSeat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::ExtIdleNotificationV1, ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
};

#[derive(Debug)]
struct AppState {
    idle_notifier: Option<ExtIdleNotifierV1>,
    seats: Vec<WlSeat>,
    toplevels: HashMap<ZwlrForeignToplevelHandleV1, ToplevelInfo>,
    db_connection: rusqlite::Connection,
}

impl AppState {
    fn new() -> anyhow::Result<AppState> {
        let database_connection = rusqlite::Connection::open("app_usage.db")?;

        database_connection.execute("PRAGMA foreign_keys = ON", ())?;

        database_connection.execute(
            "CREATE TABLE IF NOT EXISTS app_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_name TEXT NOT NULL,
                start_time INTEGER NOT NULL,
                end_time INTEGER NOT NULL,
                duration INTEGER NOT NULL
            )",
            (),
        )?;

        Ok(Self {
            idle_notifier: None,
            seats: vec![],
            toplevels: HashMap::new(),
            db_connection: database_connection,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct ToplevelInfo {
    title: Option<String>,
    app_id: Option<String>,
    focused_since: Option<Instant>,
    state: Option<Vec<zwlr_foreign_toplevel_handle_v1::State>>,
    parent: Option<Option<ZwlrForeignToplevelHandleV1>>,
    done: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        println!("event: {:?}", event);
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_idle_notifier_v1" => {
                    state.idle_notifier =
                        Some(proxy.bind::<ExtIdleNotifierV1, _, _>(name, version, qhandle, ()))
                }
                "wl_seat" => {
                    let seat = proxy.bind::<WlSeat, _, _>(name, version, qhandle, ());
                    println!("seat: {:?}", seat);
                    state.seats.push(seat);
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    proxy.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version, qhandle, ());
                }
                _ => (),
            }
        }
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtIdleNotifierV1,
        event: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // just ignore
        println!("idle notifier event: {:?}", event);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: <ZwlrForeignToplevelManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // just ignore
        println!("toplevel manager event: {:?}", event);
    }

    event_created_child!(AppState, ZwlrForeignToplevelManagerV1, [
        _ => (ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for AppState {
    fn event(
        app_state: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: <ZwlrForeignToplevelHandleV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        println!("toplevel handle event: {:?}", event);
        let item = app_state.toplevels.entry(proxy.clone()).or_default();

        use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::Event;
        match event {
            Event::Title { title } => item.title = Some(title),
            Event::AppId { app_id } => item.app_id = Some(app_id),
            Event::OutputEnter { output: _ } => (),
            Event::OutputLeave { output: _ } => (),
            Event::State { state } => {
                let new_state = state
                    .chunks_exact(4)
                    .map(|chunk| unsafe {
                        // TODO do this in a way that prevents invalid enums
                        std::mem::transmute::<_, zwlr_foreign_toplevel_handle_v1::State>(
                            u32::from_ne_bytes(chunk.try_into().unwrap()),
                        )
                    })
                    .collect::<Vec<_>>();

                let was_active = item.state.as_ref().is_some_and(|state| {
                    state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated)
                });

                let is_active =
                    new_state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated);

                // became inactive
                if was_active && !is_active {
                    println!("became inactive: {:?}", item);
                    // log time since became active
                    // remove activate time from toplevel info
                    if let Some(focused_since) = item.focused_since {
                        if let Some(ref app_id) = item.app_id {
                            let duration = Instant::now().duration_since(focused_since);
                            let now = SystemTime::now();
                            let start_time = now - duration;
                            if let Err(e) = app_state.db_connection.execute(
                                "INSERT INTO app_usage (app_name, start_time, end_time, duration) VALUES (?1, ?2, ?3, ?4)",
                                params![
                                    app_id,
                                    start_time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                    now.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                    duration.as_millis() as u64,
                                ],
                            ) {
                                println!("db insert failed: {e}");
                            }
                        }
                    }
                    item.focused_since = None;
                }

                // became active
                if is_active && !was_active {
                    println!("became active: {:?}", item);
                    item.focused_since = Some(Instant::now());
                }

                item.state = Some(new_state);
            }
            Event::Done => item.done = true,
            Event::Closed => {
                let is_active = item.state.as_ref().is_some_and(|state| {
                    state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated)
                });

                if is_active {
                    println!("active client destroyed: {:?}", item);
                    if let Some(focused_since) = item.focused_since {
                        if let Some(ref app_id) = item.app_id {
                            let duration = Instant::now().duration_since(focused_since);
                            let now = SystemTime::now();
                            let start_time = now - duration;
                            if let Err(e) = app_state.db_connection.execute(
                                "INSERT INTO app_usage (app_name, start_time, end_time, duration) VALUES (?1, ?2, ?3, ?4)",
                                params![
                                    app_id,
                                    start_time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                    now.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                    duration.as_millis() as u64,
                                ],
                            ) {
                                println!("db insert failed: {e}");
                            }
                        }
                    }
                }
                app_state.toplevels.remove(&proxy.clone());
            }
            Event::Parent { parent } => item.parent = Some(parent),
            _ => (),
        }
    }
}

impl Dispatch<WlSeat, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        event: <WlSeat as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // just ignore
        println!("seat event: {:?}", event);
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: <ExtIdleNotificationV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        println!("idle notification event: {:?}", event);
        // println!("toplevels: {:#?}", state.toplevels);
        use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::Event;
        match event {
            Event::Idled => {
                // log active time, reset active_since number
                for toplevel in state
                    .toplevels
                    .values_mut()
                    .filter(|toplevel| toplevel.focused_since.is_some())
                {
                    println!(
                        "idleing, logging active duration for toplevel: {:?}",
                        toplevel
                    );
                    if let Some(ref app_id) = toplevel.app_id {
                        let duration =
                            Instant::now().duration_since(toplevel.focused_since.unwrap());
                        let now = SystemTime::now();
                        let start_time = now - duration;
                        if let Err(e) = state.db_connection.execute(
                            "INSERT INTO app_usage (app_name, start_time, end_time, duration) VALUES (?1, ?2, ?3, ?4)",
                            params![
                                app_id,
                                start_time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                now.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                duration.as_millis() as u64,
                            ],
                        ) {
                            println!("db insert failed: {e}");
                        }
                    }
                    toplevel.focused_since = None;
                }
            }
            Event::Resumed => {
                for toplevel in state.toplevels.values_mut().filter(|toplevel| {
                    toplevel.state.as_ref().is_some_and(|state| {
                        state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated)
                    })
                }) {
                    toplevel.focused_since = Some(Instant::now());
                }
            }
            _ => unreachable!(),
        }
    }
}

fn main() -> anyhow::Result<()> {
    println!("Hello, world!");
    let wayland_connection = wayland_client::Connection::connect_to_env()?;

    let mut queue = {
        let display = wayland_connection.display();

        let queue = wayland_connection.new_event_queue();
        let queue_handle = queue.handle();

        display.get_registry(&queue_handle, ());

        queue
    };

    let mut state = AppState::new()?;

    queue.roundtrip(&mut state)?;

    state.idle_notifier.clone().unwrap().get_idle_notification(
        5_000,
        &state.seats[0],
        &queue.handle(),
        (),
    );

    loop {
        queue.blocking_dispatch(&mut state)?;
        println!("dispatched!");
    }
}
