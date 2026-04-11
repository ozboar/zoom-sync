//! Utilities for getting weather info

use std::error::Error;

use bpaf::Bpaf;
use chrono::Timelike;
use ipinfo::IpInfo;
use open_meteo_api::query::OpenMeteo;
use zoom_sync_core::Board;

#[derive(Clone, Debug, Bpaf)]
#[bpaf(adjacent)]
pub struct Coords {
    /// Optional coordinates to use for fetching weather data, skipping ipinfo geolocation api.
    #[bpaf(long)]
    #[allow(dead_code)]
    pub coords: (),
    /// Latitude
    #[bpaf(positional("LAT"))]
    pub lat: f32,
    /// Longitude
    #[bpaf(positional("LON"))]
    pub long: f32,
}

/// Weather forecast options:
#[derive(Clone, Debug, Bpaf)]
pub enum WeatherArgs {
    /// Disable updating weather info completely
    #[bpaf(long("no-weather"))]
    Disabled,
    // default
    Auto {
        #[bpaf(external, optional)]
        coords: Option<Coords>,
    },
    #[bpaf(adjacent)]
    Manual {
        /// Manually provide weather data, skipping open-meteo weather api. All values are
        /// unitless.
        #[bpaf(short, long)]
        #[allow(dead_code)]
        weather: (),
        /// WMO Index
        #[bpaf(positional("WMO"))]
        wmo: u8,
        /// Current temperature
        #[bpaf(positional("CUR"))]
        current: i16,
        /// Minumum temperature
        #[bpaf(positional("MIN"))]
        min: i16,
        /// Maximum temperature
        #[bpaf(positional("MAX"))]
        max: i16,
    },
}

pub async fn get_coords() -> Result<(f32, f32), Box<dyn Error>> {
    println!("fetching geolocation from ipinfo ...");
    let mut ipinfo = IpInfo::new(ipinfo::IpInfoConfig {
        token: None,
        ..Default::default()
    })?;
    let info = ipinfo.lookup_self_v4().await?;
    let (lat, long) = info.loc.split_once(',').unwrap();
    Ok((lat.parse().unwrap(), long.parse().unwrap()))
}

/// Weather data from API
pub struct WeatherData {
    pub wmo: u8,
    pub is_day: bool,
    pub current: f32,
    pub min: f32,
    pub max: f32,
}

/// Get the current weather, using ipinfo for geolocation, and open-meteo for forcasting
pub async fn get_weather(
    lat: f32,
    long: f32,
    fahrenheit: bool,
) -> Result<WeatherData, Box<dyn Error>> {
    println!("fetching current weather from open-meteo for [{lat}, {long}] ...");
    let res = OpenMeteo::new()
        .coordinates(lat, long)?
        .current_weather()?
        .time_zone(open_meteo_api::models::TimeZone::Auto)?
        .daily()?
        .query()
        .await?;

    let current = res.current_weather.unwrap();
    let wmo = current.weathercode as u8;
    let is_day = current.is_day == 1.0;

    let daily = res.daily.unwrap();
    let mut min = daily.temperature_2m_min.first().unwrap().unwrap();
    let mut max = daily.temperature_2m_max.first().unwrap().unwrap();
    let mut temp = current.temperature;

    if fahrenheit {
        min = min * 9. / 5. + 32.;
        max = max * 9. / 5. + 32.;
        temp = temp * 9. / 5. + 32.;
    }

    Ok(WeatherData {
        wmo,
        is_day,
        current: temp,
        min,
        max,
    })
}

pub async fn apply_weather(
    board: &mut dyn Board,
    args: &mut WeatherArgs,
    farenheit: bool,
) -> Result<(), Box<dyn Error>> {
    let weather = board.as_weather().ok_or("board does not support weather")?;

    match args {
        WeatherArgs::Disabled => println!("skipping weather"),
        WeatherArgs::Auto { coords } => {
            // attempt to backfill coordinates if not provided
            if coords.is_none() {
                match get_coords().await {
                    Ok((lat, long)) => {
                        *coords = Some(Coords {
                            coords: (),
                            lat,
                            long,
                        })
                    },
                    Err(e) => eprintln!("warning: failed to fetch geolocation from ipinfo: {e}"),
                }
            }

            // try to update weather if we have some coordinates
            if let Some(Coords { lat, long, .. }) = *coords {
                match get_weather(lat, long, farenheit).await {
                    Ok(data) => {
                        weather
                            .set_weather(
                                data.wmo,
                                data.is_day,
                                data.current.round() as i16,
                                data.min.round() as i16,
                                data.max.round() as i16,
                            )
                            .map_err(|e| format!("failed to set weather: {e}"))?;
                        println!(
                            "updated weather {{ wmo: {}, is_day: {}, current: {}, min: {}, max: {} }}",
                            data.wmo, data.is_day, data.current, data.min, data.max
                        );
                    },
                    Err(e) => eprintln!("failed to fetch weather, skipping: {e}"),
                }
            }
        },
        WeatherArgs::Manual {
            wmo,
            current,
            min,
            max,
            ..
        } => {
            let hour = chrono::Local::now().hour();
            let is_day = (6..=18).contains(&hour);
            weather.set_weather(*wmo, is_day, *current, *min, *max)?;
        },
    }

    Ok(())
}
