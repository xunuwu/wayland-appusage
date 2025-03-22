use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::params;
use tracing::{info, trace, warn};
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
pub struct AppState {
    pub idle_notifier: Option<ExtIdleNotifierV1>,
    pub toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    pub seats: Vec<WlSeat>,
    toplevels: HashMap<ZwlrForeignToplevelHandleV1, ToplevelInfo>,
    db_connection: rusqlite::Connection,
}

#[derive(Debug, Clone, Default)]
struct ToplevelInfo {
    app_id: Option<String>,
    focused_since: Option<Instant>,
    state: Option<Vec<zwlr_foreign_toplevel_handle_v1::State>>,
}

fn insert_usage(
    conn: &rusqlite::Connection,
    app_name: String,
    end_time: SystemTime,
    duration: Duration,
) -> Result<usize, rusqlite::Error> {
    let start_time = (end_time - duration).duration_since(UNIX_EPOCH).unwrap();

    conn.execute(
        "INSERT INTO app_usage (app_name, start_time, end_time, duration) VALUES (?1, ?2, ?3, ?4)",
        params![
            app_name,
            start_time.as_millis() as u64,
            end_time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            duration.as_millis() as u64,
        ],
    )
}

impl AppState {
    pub fn new() -> anyhow::Result<AppState> {
        let db_path = xdg::BaseDirectories::with_prefix("wayland-appusage")?
            .place_data_file("app_usage.db")?;
        let database_connection = rusqlite::Connection::open(db_path)?;

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
            toplevel_manager: None,
            seats: vec![],
            toplevels: HashMap::new(),
            db_connection: database_connection,
        })
    }
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
        trace!("event: {:?}", event);
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
                    state.seats.push(seat);
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.toplevel_manager =
                        Some(proxy.bind::<ZwlrForeignToplevelManagerV1, _, _>(
                            name,
                            version,
                            qhandle,
                            (),
                        ));
                }
                _ => (),
            }
        }
    }
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
        trace!("toplevel handle event: {:?}", event);
        let item = app_state.toplevels.entry(proxy.clone()).or_default();

        use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::Event;
        match event {
            Event::AppId { app_id } => item.app_id = Some(app_id),
            Event::State { state } => {
                let new_state = state
                    .chunks_exact(4)
                    .map(|chunk| {
                        let raw_value = u32::from_ne_bytes(chunk.try_into().unwrap());
                        zwlr_foreign_toplevel_handle_v1::State::try_from(raw_value).unwrap()
                    })
                    .collect::<Vec<_>>();

                let was_active = item.state.as_ref().is_some_and(|state| {
                    state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated)
                });

                let is_active =
                    new_state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated);

                // became inactive
                if was_active && !is_active {
                    info!("became inactive:{:?}", item.app_id);
                    // log time since became active
                    // remove activate time from toplevel info
                    if let Some(focused_since) = item.focused_since {
                        if let Some(ref app_id) = item.app_id {
                            let duration = Instant::now().duration_since(focused_since);
                            let now = SystemTime::now();
                            if let Err(e) = insert_usage(
                                &app_state.db_connection,
                                app_id.to_string(),
                                now,
                                duration,
                            ) {
                                warn!("db insert failed: {e}");
                            }
                        }
                    }
                    item.focused_since = None;
                }

                // became active
                if is_active && !was_active {
                    trace!("became active: {:?}", item);
                    info!("became active: {:?}", item.app_id);
                    item.focused_since = Some(Instant::now());
                }

                item.state = Some(new_state);
            }
            Event::Closed => {
                let is_active = item.state.as_ref().is_some_and(|state| {
                    state.contains(&zwlr_foreign_toplevel_handle_v1::State::Activated)
                });

                if is_active {
                    info!("active client destroyed: {:?}", item);
                    if let Some(focused_since) = item.focused_since {
                        if let Some(ref app_id) = item.app_id {
                            let duration = Instant::now().duration_since(focused_since);
                            let now = SystemTime::now();
                            if let Err(e) = insert_usage(
                                &app_state.db_connection,
                                app_id.to_string(),
                                now,
                                duration,
                            ) {
                                warn!("db insert failed: {e}");
                            }
                        }
                    }
                }
                app_state.toplevels.remove(&proxy.clone());
            }
            _ => (),
        }
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
        trace!("idle notification event: {:?}", event);
        use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::Event;
        match event {
            Event::Idled => {
                // log active time, reset active_since number
                for toplevel in state
                    .toplevels
                    .values_mut()
                    .filter(|toplevel| toplevel.focused_since.is_some())
                {
                    info!(
                        "idleing, logging active duration for toplevel: {:?}",
                        toplevel.app_id
                    );
                    if let Some(ref app_id) = toplevel.app_id {
                        let duration =
                            Instant::now().duration_since(toplevel.focused_since.unwrap());
                        let now = SystemTime::now();
                        if let Err(e) =
                            insert_usage(&state.db_connection, app_id.to_string(), now, duration)
                        {
                            warn!("db insert failed: {e}");
                        }
                    }
                    toplevel.focused_since = None;
                }
            }
            Event::Resumed => {
                info!("resumed");
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

// ignore
impl Dispatch<ExtIdleNotifierV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtIdleNotifierV1,
        _event: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlSeat, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        _event: <ZwlrForeignToplevelManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }

    event_created_child!(AppState, ZwlrForeignToplevelManagerV1, [
        _ => (ZwlrForeignToplevelHandleV1, ())
    ]);
}
