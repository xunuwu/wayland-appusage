use std::{error::Error, io, time};

use chrono::{Datelike, Local};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Text,
    widgets::{
        Bar, BarChart, BarGroup, Block, Borders, List, ListItem, ListState, Paragraph, Widget,
    },
};
use rusqlite::Connection;

mod db;

pub struct App {
    exit: bool,
    connection: Connection,
    app_list: AppList,
}

struct AppList {
    items: Vec<(String, u64)>,
    time_to_show: AppListTime,
    state: ListState,
}

#[derive(Default)]
enum AppListTime {
    #[default]
    Today,
    ThisWeek,
    ThisMonth,
    AllTime,
}

impl AppListTime {
    fn next(&self) -> Self {
        match self {
            AppListTime::Today => AppListTime::Today,
            AppListTime::ThisWeek => AppListTime::Today,
            AppListTime::ThisMonth => AppListTime::ThisWeek,
            AppListTime::AllTime => AppListTime::ThisMonth,
        }
    }

    fn prev(&self) -> Self {
        match self {
            AppListTime::Today => AppListTime::ThisWeek,
            AppListTime::ThisWeek => AppListTime::ThisMonth,
            AppListTime::ThisMonth => AppListTime::AllTime,
            AppListTime::AllTime => AppListTime::AllTime,
        }
    }

    fn timestamps(&self) -> Option<(u64, u64)> {
        let now = Local::now();
        let start_of_today = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
        let end_of_today = start_of_today + chrono::Duration::days(1);

        match self {
            AppListTime::Today => Some((
                start_of_today.and_utc().timestamp_millis() as u64,
                end_of_today.and_utc().timestamp_millis() as u64,
            )),
            AppListTime::ThisWeek => {
                let one_week_ago = end_of_today - chrono::Duration::weeks(1);
                Some((
                    one_week_ago.and_utc().timestamp_millis() as u64,
                    end_of_today.and_utc().timestamp_millis() as u64,
                ))
            }
            AppListTime::ThisMonth => {
                let one_month_ago = end_of_today - chrono::Duration::weeks(4);
                Some((
                    one_month_ago.and_utc().timestamp_millis() as u64,
                    end_of_today.and_utc().timestamp_millis() as u64,
                ))
            }
            AppListTime::AllTime => None,
        }
    }
}

impl std::fmt::Display for AppListTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                AppListTime::Today => "Today",
                AppListTime::ThisWeek => "Last Week",
                AppListTime::ThisMonth => "Last Month",
                AppListTime::AllTime => "All Time",
            }
        )
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut terminal = ratatui::init();
    let app_result = App::new().run(&mut terminal);
    ratatui::restore();

    Ok(app_result?)
}

impl App {
    fn new() -> Self {
        let db_path = xdg::BaseDirectories::with_prefix("wayland-appusage")
            .unwrap()
            .place_data_file("app_usage.db")
            .unwrap();
        let conn = Connection::open(db_path).unwrap();
        let time_to_show = AppListTime::default();
        let apps = db::list_apps(&conn, time_to_show.timestamps()).unwrap();

        Self {
            exit: false,
            connection: conn,
            app_list: AppList {
                items: apps,
                state: ListState::default(),
                time_to_show,
            },
        }
    }
}

impl App {
    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn refetch_applist(&mut self) {
        self.app_list.items =
            db::list_apps(&self.connection, self.app_list.time_to_show.timestamps()).unwrap();
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.render(frame.area(), frame.buffer_mut());
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char('q') => self.exit(),
                    KeyCode::Char('j') | KeyCode::Down => self.app_list.state.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => self.app_list.state.select_previous(),
                    KeyCode::Char('g') | KeyCode::Home => self.app_list.state.select_first(),
                    KeyCode::Char('G') | KeyCode::End => self.app_list.state.select_last(),
                    KeyCode::Char('h') | KeyCode::Left => {
                        self.app_list.time_to_show = self.app_list.time_to_show.prev();
                        self.refetch_applist();
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        self.app_list.time_to_show = self.app_list.time_to_show.next();
                        self.refetch_applist();
                    }
                    _ => {}
                }
            }
            _ => {}
        };
        Ok(())
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn get_week_data(&self) -> Vec<(String, u64)> {
        let now = Local::now();
        let start_of_today = now.date_naive().and_hms_opt(0, 0, 0).unwrap();

        // TODO cache this!!!
        (0..7)
            .map(|i| {
                let day = start_of_today - chrono::Duration::days(i);
                (
                    day.weekday().to_string(),
                    db::get_data_for_time(
                        &self.connection,
                        (
                            day.and_utc().timestamp_millis() as u64,
                            (day + chrono::Duration::days(1))
                                .and_utc()
                                .timestamp_millis() as u64,
                        ),
                    )
                    .unwrap(),
                )
            })
            .collect()
    }

    fn render_bars(&mut self, week_data: Vec<(String, u64)>, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Past Week");

        let width = block.inner(area).width;
        let gap_size = 2;
        let item_count = 7;
        let total_reserved = gap_size * (item_count - 1) + 2;
        let space_per_item = (width - total_reserved) / item_count;

        let bars: Vec<_> = week_data
            .iter()
            .map(|(day, value)| {
                Bar::default()
                    .value(*value)
                    .label(day.clone().into())
                    .text_value(
                        humantime::format_duration(time::Duration::from_secs(*value / 1000))
                            .to_string(),
                    )
            })
            .rev()
            .collect();

        BarChart::default()
            .block(block)
            .data(BarGroup::default().bars(&bars))
            .bar_width(space_per_item)
            .bar_gap(gap_size)
            .direction(Direction::Vertical)
            .render(area, buf);
    }

    fn render_legend(&mut self, week_data: Vec<(String, u64)>, area: Rect, buf: &mut Buffer) {
        let legend_items = week_data
            .iter()
            .map(|(day, value)| {
                ListItem::new(format!(
                    "{day}: {}",
                    // TODO exclude seconds here, only show hours and minutes
                    humantime::format_duration(time::Duration::from_secs(*value / 1000))
                ))
            })
            .rev();

        List::new(legend_items)
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let name_items = self
            .app_list
            .items
            .iter()
            .map(|x| x.0.clone())
            .collect::<Vec<_>>();

        let time_items = self
            .app_list
            .items
            .iter()
            .map(|x| {
                ListItem::new(
                    Text::from(
                        humantime::format_duration(time::Duration::from_secs(x.1 / 1000))
                            .to_string(),
                    )
                    .right_aligned(),
                )
            })
            .collect::<Vec<_>>();

        let [name_list, time_list] = [List::new(name_items), List::new(time_items)].map(|x| {
            x.block(
                Block::default()
                    .borders(Borders::ALL)
                    .title_alignment(Alignment::Center)
                    .title(format!("Top {}", self.app_list.time_to_show)),
            )
            .highlight_symbol(">")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
        });

        ratatui::widgets::StatefulWidget::render(time_list, area, buf, &mut self.app_list.state);
        ratatui::widgets::StatefulWidget::render(name_list, area, buf, &mut self.app_list.state);
    }

    fn render_item(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(selected_num) = self.app_list.state.selected() else {
            return;
        };

        let selected_app = self.app_list.items[selected_num].clone();

        // Line::from(selected_app).render(area, buf);
        let block = Block::new()
            .borders(Borders::ALL)
            .title(selected_app.0.clone());

        let inner = block.inner(area);

        let now = Local::now();
        let start_of_today = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
        let end_of_today = start_of_today + chrono::Duration::days(1);

        let usage_today = db::get_data_for_app_and_time(
            &self.connection,
            selected_app.0.clone(),
            (
                start_of_today.and_utc().timestamp_millis() as u64,
                end_of_today.and_utc().timestamp_millis() as u64,
            ),
        )
        .unwrap();

        let one_week_ago = end_of_today - chrono::Duration::weeks(1);

        let usage_this_wek = db::get_data_for_app_and_time(
            &self.connection,
            selected_app.0.clone(),
            (
                one_week_ago.and_utc().timestamp_millis() as u64,
                end_of_today.and_utc().timestamp_millis() as u64,
            ),
        )
        .unwrap();

        let usage_all_time = db::get_total_app_usage(&self.connection, selected_app.0).unwrap();

        Paragraph::new(format!(
            "Today: {}\nThis week: {}\nAll time: {}",
            humantime::format_duration(time::Duration::from_secs(usage_today / 1000)),
            humantime::format_duration(time::Duration::from_secs(usage_this_wek / 1000)),
            humantime::format_duration(time::Duration::from_secs(usage_all_time / 1000)),
        ))
        .render(inner, buf);

        block.render(area, buf);
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [top_area, bottom_area] =
            Layout::vertical([Constraint::Max(20), Constraint::Fill(1)]).areas(area);
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(bottom_area);

        // let [chart_area, list_area] =
        //     Layout::vertical([Constraint::Min(20), Constraint::Percentage(100)]).areas(left_area);

        let week_data = self.get_week_data();
        self.render_bars(week_data.clone(), top_area, buf);
        // self.render_bars(week_data.clone(), chart_area, buf);
        // self.render_legend(week_data, legend_area, buf);

        self.render_list(left_area, buf);
        self.render_item(right_area, buf);
    }
}
