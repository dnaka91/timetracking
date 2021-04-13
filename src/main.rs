use chrono::{prelude::*, serde::ts_seconds, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use iif::iif;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(feature = "binary")]
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Options {
    #[structopt(subcommand)]
    command: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// show info from the latest entry
    Status,

    /// start time tracking
    Start {
        /// a description for the event
        description: Option<String>,

        /// the time at which the event happend.
        /// format: "HH:MM:SS" or "YY-mm-dd HH:MM:SS" [defaults to current time]
        #[structopt(short, long)]
        at: Option<String>,
    },

    /// stop time tracking
    Stop {
        /// a description for the event
        description: Option<String>,

        /// the time at which the event happend.
        /// format: "HH:MM:SS" or "YY-mm-dd HH:MM:SS" [defaults to current time]
        #[structopt(short, long)]
        at: Option<String>,
    },

    /// continue time tracking with last description
    Continue,

    /// list all entries
    List,

    /// show path to data file
    Path,

    /// show work time for given timespan
    Show {
        /// the start time [defaults to current day 00:00:00]
        #[structopt(short, long)]
        from: Option<String>,

        /// the stop time [defaults to start day 23:59:59]
        #[structopt(short, long)]
        to: Option<String>,

        /// include seconds in time calculation
        #[structopt(short)]
        include_seconds: bool,

        /// filter entries. possible filter values: "week" or part of the description
        filter: Option<String>,
    },

    #[cfg(feature = "binary")]
    /// export the file as json
    Export {
        /// where to write the json file
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrackingData {
    description: Option<String>,

    #[serde(with = "ts_seconds")]
    time: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum TrackingEvent {
    Start(TrackingData),
    Stop(TrackingData),
}

impl TrackingEvent {
    fn time(&self, include_seconds: bool) -> DateTime<Utc> {
        match self {
            Self::Start(TrackingData { time, .. }) | Self::Stop(TrackingData { time, .. }) => {
                let time = *time;
                if include_seconds {
                    time
                } else {
                    time.with_second(0).expect("could not set seconds to zero")
                }
            }
        }
    }

    fn description(&self) -> Option<String> {
        match self {
            Self::Start(TrackingData { description, .. })
            | Self::Stop(TrackingData { description, .. }) => description.clone(),
        }
    }

    fn is_start(&self) -> bool {
        match self {
            Self::Start(_) => true,
            Self::Stop(_) => false,
        }
    }

    fn is_stop(&self) -> bool {
        match self {
            Self::Start(_) => false,
            Self::Stop(_) => true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DateOrDateTime {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

#[cfg(feature = "binary")]
fn read_data<P: AsRef<Path>>(path: P) -> Vec<TrackingEvent> {
    let data = std::fs::read(&path).unwrap_or_default();
    bincode::deserialize(&data).unwrap_or_default()
}

#[cfg(not(feature = "binary"))]
fn read_data<P: AsRef<Path>>(path: P) -> Vec<TrackingEvent> {
    let data = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&data).unwrap_or_default()
}

#[cfg(feature = "binary")]
fn write_data<P: AsRef<Path>>(path: P, data: &[TrackingEvent]) {
    let data = bincode::serialize(data).expect("could not serialize data");
    std::fs::write(path, data).expect("could not write data file");
}

fn write_data_json<P: AsRef<Path>>(path: P, data: &[TrackingEvent]) {
    let data = serde_json::to_string(data).expect("could not serialize data");
    std::fs::write(path, data).expect("could not write data file");
}

#[cfg(not(feature = "binary"))]
fn write_data<P: AsRef<Path>>(path: P, data: &[TrackingEvent]) {
    write_data_json(path, data);
}

fn start_tracking(data: &mut Vec<TrackingEvent>, description: Option<String>, at: Option<String>) {
    let should_add = match data.last() {
        None => true,
        Some(event) => event.is_stop(),
    };
    if should_add {
        data.push(TrackingEvent::Start(TrackingData {
            description,
            time: at.map_or_else(|| Local::now().into(), |at| parse_date_time(&at)),
        }));
    }
}

fn stop_tracking(data: &mut Vec<TrackingEvent>, description: Option<String>, at: Option<String>) {
    let should_add = match data.last() {
        None => true,
        Some(event) => event.is_start(),
    };
    if should_add {
        data.push(TrackingEvent::Stop(TrackingData {
            description,
            time: at.map_or_else(|| Local::now().into(), |at| parse_date_time(&at)),
        }))
    }
}

fn continue_tracking(data: &mut Vec<TrackingEvent>) {
    if let Some(TrackingEvent::Stop { .. }) = data.last() {
        if let Some(TrackingEvent::Start(TrackingData { description, .. })) =
            data.iter().rev().find(|t| t.is_start()).cloned()
        {
            data.push(TrackingEvent::Start(TrackingData {
                description,
                time: Local::now().into(),
            }))
        }
    } else {
        eprintln!("Time tracking couldn't be continued, because there are no entries. Use the start command instead!");
    }
}

fn split_duration(duration: Duration) -> (i64, i64, i64) {
    let hours = duration.num_hours();
    let hours_in_minutes = hours * 60;
    let hours_in_seconds = hours_in_minutes * 60;
    let minutes = duration.num_minutes() - hours_in_minutes;
    let minutes_in_seconds = minutes * 60;
    let seconds = duration.num_seconds() - hours_in_seconds - minutes_in_seconds;
    (hours, minutes, seconds)
}

fn show(
    data: &[TrackingEvent],
    from: Option<String>,
    to: Option<String>,
    filter: Option<String>,
    include_seconds: bool,
) -> Option<()> {
    let (filter, from, to) = match filter {
        Some(from) if from == "week" => {
            let now = Local::today();
            let weekday = now.weekday();
            let offset = weekday.num_days_from_monday();
            let (monday_offset, sunday_offset) = (offset, 6 - offset);
            let from = DateOrDateTime::Date(now.with_day(now.day() - monday_offset)?.naive_local());
            let to = DateOrDateTime::Date(now.with_day(now.day() + sunday_offset)?.naive_local());
            (None, Some(from), Some(to))
        }
        f => {
            let from = match &from {
                Some(s) => Some(parse_date_or_date_time(&s)),
                None => None,
            }.unwrap_or_else(||DateOrDateTime::Date(Local::today().naive_local()));

            let to = match to {
                Some(s) => parse_date_or_date_time(&s),
                None => match from {
                    DateOrDateTime::DateTime(from) => DateOrDateTime::Date(from.date()),
                    from => from,
                },
            };
            (f, Some(from), Some(to))
        }
    };
    let mut data_iterator = data
        .iter()
        .filter(|entry| iif!(filter.clone().unwrap_or_default() == "all", true, match from {
            None => true,
            Some(DateOrDateTime::Date(from)) => {
                entry.time(true).timestamp_millis()
                    >= TimeZone::from_local_date(&Local, &from)
                        .unwrap()
                        .and_time(NaiveTime::from_hms(0, 0, 0))
                        .unwrap()
                        .timestamp_millis()
            }
            Some(DateOrDateTime::DateTime(from)) => {
                entry.time(true).timestamp_millis()
                    >= TimeZone::from_local_datetime(&Local, &from)
                        .unwrap()
                        .timestamp_millis()
            }
        }))
        .filter(|entry| iif!(filter.clone().unwrap_or_default() == "all", true, match to {
            None => true,
            Some(DateOrDateTime::Date(to)) => {
                entry.time(true).timestamp_millis()
                    <= TimeZone::from_local_date(&Local, &to)
                        .unwrap()
                        .and_time(NaiveTime::from_hms(23, 59, 59))
                        .unwrap()
                        .timestamp_millis()
            }
            Some(DateOrDateTime::DateTime(to)) => {
                entry.time(true).timestamp_millis()
                    <= TimeZone::from_local_datetime(&Local, &to)
                        .unwrap()
                        .timestamp_millis()
            }
        }))
        .filter(|entry| match entry {
            TrackingEvent::Start(TrackingData { description, .. })
            | TrackingEvent::Stop(TrackingData { description, .. }) => match (&filter, description)
            {
                (Some(filter), Some(description)) => filter == "all" || description.contains(filter),
                (Some(filter), None) => filter == "all",
                (None, _) => true,
            },
        })
        .skip_while(|entry| TrackingEvent::is_stop(entry));
    let mut work_day = Duration::zero();
    loop {
        let start = data_iterator.next();
        let stop = data_iterator.next();
        match (start, stop) {
            (Some(start), Some(stop)) => {
                let duration = stop.time(include_seconds) - start.time(include_seconds);
                work_day = work_day
                    .checked_add(&duration)
                    .expect("couldn't add up durations");
            }
            (Some(start), None) => {
                let now = if include_seconds {
                    Utc::now()
                } else {
                    Utc::now().with_second(0).unwrap()
                };
                let duration = now - start.time(include_seconds);
                work_day = work_day
                    .checked_add(&duration)
                    .expect("couldn't add up durations");
                break;
            }
            (_, _) => break,
        }
    }
    let (hours, minutes, seconds) = split_duration(work_day);
    println!("Work Time: {:02}:{:02}:{:02}", hours, minutes, seconds);
    Some(())
}

fn status(data: &[TrackingEvent]) {
    if let Some(event) = data.last() {
        let time = event.time(true).with_timezone(&Local);
        let active = event.is_start();
        let text = iif!(active, "Start", "End");
        if let Some(description) = event.description() {
            println!("Description: {}", description,);
            println!("Active: {}", active);
            println!(
                "{} Time: {:02}:{:02}:{:02}",
                text,
                time.hour(),
                time.minute(),
                time.second()
            );
        } else {
            println!("Active: {}", active);
            println!(
                "{} Time: {:02}:{:02}:{:02}",
                text,
                time.hour(),
                time.minute(),
                time.second()
            );
        }
    }
}

fn main() {
    let Options { command } = Options::from_args();

    let mut path = dirs::home_dir().unwrap_or_else(|| ".".into());

    if cfg!(feature = "binary") {
        path.push("timetracking.bin");
    } else {
        path.push("timetracking.json");
    }

    let mut data = read_data(&path);

    match command {
        Command::Start { description, at } => start_tracking(&mut data, description, at),
        Command::Stop { description, at } => stop_tracking(&mut data, description, at),
        Command::Continue => continue_tracking(&mut data),
        Command::List => data.iter().for_each(|e| println!("{:?}", e)),
        Command::Path => println!("{}", path.to_string_lossy()),
        Command::Show {
            from,
            to,
            filter,
            include_seconds,
        } => show(&data, from, to, filter, include_seconds).unwrap(),
        Command::Status => status(&data),
        #[cfg(feature = "binary")]
        Command::Export { path } => {
            write_data_json(path, &data);
        }
        #[allow(unreachable_patterns)]
        _ => unimplemented!(),
    }

    write_data(path, &data);
}

fn parse_date_time(s: &str) -> DateTime<Utc> {
    if let Ok(time) = NaiveTime::parse_from_str(s, "%H:%M:%S") {
        let today = Local::today();
        let date_time = today.and_time(time).unwrap();
        return date_time.with_timezone(&Utc);
    }
    if let Ok(time) = NaiveTime::parse_from_str(&format!("{}:0", s), "%H:%M:%S") {
        let today = Local::today();
        let date_time = today.and_time(time).unwrap();
        return date_time.with_timezone(&Utc);
    }
    if let Ok(time) = NaiveTime::parse_from_str(&format!("{}:0:0", s), "%H:%M:%S") {
        let today = Local::today();
        let date_time = today.and_time(time).unwrap();
        return date_time.with_timezone(&Utc);
    }
    if let Ok(date_time) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return TimeZone::from_local_datetime(&Local, &date_time)
            .unwrap()
            .with_timezone(&Utc);
    }
    if let Ok(date_time) = NaiveDateTime::parse_from_str(&format!("{}:0", s), "%Y-%m-%d %H:%M:%S") {
        return TimeZone::from_local_datetime(&Local, &date_time)
            .unwrap()
            .with_timezone(&Utc);
    }
    let date_time =
        NaiveDateTime::parse_from_str(&format!("{}:0:0", s), "%Y-%m-%d %H:%M:%S").unwrap();
    TimeZone::from_local_datetime(&Local, &date_time)
        .unwrap()
        .with_timezone(&Utc)
}

fn parse_date_or_date_time(s: &str) -> DateOrDateTime {
    if let Ok(date) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return DateOrDateTime::Date(date);
    }
    if let Ok(date) =
        NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").map(DateOrDateTime::DateTime)
    {
        return date;
    }
    if let Ok(date) = NaiveTime::parse_from_str(&s, "%H:%M:%S")
        .map(|time| Local::today().and_time(time).unwrap())
        .map(|date_time| date_time.naive_local())
        .map(DateOrDateTime::DateTime)
    {
        return date;
    }
    if let Ok(date) = NaiveTime::parse_from_str(&format!("{}:0", s), "%H:%M:%S")
        .map(|time| Local::today().and_time(time).unwrap())
        .map(|date_time| date_time.naive_local())
        .map(DateOrDateTime::DateTime)
    {
        return date;
    }
    if let Ok(date) = NaiveTime::parse_from_str(&format!("{}:0:0", s), "%H:%M:%S")
        .map(|time| Local::today().and_time(time).unwrap())
        .map(|date_time| date_time.naive_local())
        .map(DateOrDateTime::DateTime)
    {
        return date;
    }
    if let Ok(date) = NaiveDateTime::parse_from_str(&format!("{}:0", s), "%Y-%m-%d %H:%M:%S")
        .map(DateOrDateTime::DateTime)
    {
        return date;
    }
    NaiveDateTime::parse_from_str(&format!("{}:0:0", s), "%Y-%m-%d %H:%M:%S")
        .map(DateOrDateTime::DateTime)
        .unwrap()
}
