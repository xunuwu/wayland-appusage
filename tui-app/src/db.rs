use rusqlite::{params, Connection};

pub fn list_apps(
    conn: &Connection,
    time_range: Option<(u64, u64)>,
) -> Result<Vec<(String, u64)>, rusqlite::Error> {
    if let Some((start_time, end_time)) = time_range {
        let mut stmt = conn.prepare(
            "select app_name, sum(duration) as total_duration
         from app_usage
         where start_time >= ? and start_time < ?
         group by app_name
         order by total_duration desc",
        )?;
        let x = stmt
            .query_map([start_time, end_time], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?
            .collect();
        x
    } else {
        let mut stmt = conn.prepare(
            "select app_name, sum(duration)
         from app_usage
         group by app_name
         order by sum(duration) desc",
        )?;
        let x = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?
            .collect();
        x
    }
}

pub fn get_data_for_app_and_time(
    conn: &Connection,
    app_name: String,
    (start_time, end_time): (u64, u64),
) -> Result<u64, rusqlite::Error> {
    conn.query_row(
        "select sum(duration)
            from app_usage
            where app_name == ? and start_time >= ? and start_time < ?",
        params![app_name, start_time, end_time],
        |row| {
            // println!("row!!: {:?}", row.get::<_, u64>(0).or_else(|_| Ok(0)));
            Ok(row.get::<_, u64>(0).unwrap_or(0))
        },
    )
}

pub fn get_total_app_usage(conn: &Connection, app_name: String) -> Result<u64, rusqlite::Error> {
    conn.query_row(
        "select sum(duration)
            from app_usage
            where app_name == ?",
        [app_name],
        |row| {
            // println!("row!!: {:?}", row.get::<_, u64>(0).or_else(|_| Ok(0)));
            Ok(row.get::<_, u64>(0).unwrap_or(0))
        },
    )
}

pub fn get_data_for_time(
    conn: &Connection,
    (start_time, end_time): (u64, u64),
) -> Result<u64, rusqlite::Error> {
    conn.query_row(
        "select sum(duration)
            from app_usage
            where start_time >= ? and start_time < ?",
        [start_time, end_time],
        |row| {
            // println!("row!!: {:?}", row.get::<_, u64>(0).or_else(|_| Ok(0)));
            Ok(row.get::<_, u64>(0).unwrap_or(0))
        },
    )
}
