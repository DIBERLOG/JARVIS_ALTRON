use serde::{Deserialize, Serialize};
use std::time::Duration;

fn weather_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "Погодный сервис отвечает слишком долго. Попробуйте чуть позже."
    } else if error.status().is_some_and(|status| status.as_u16() == 429) {
        "Погодный сервис временно ограничил запросы. Попробуйте через несколько минут."
    } else if error.status().is_some_and(|status| status.is_server_error()) {
        "У погодного сервиса временный сбой. Попробуйте обновить прогноз чуть позже."
    } else if error.is_connect() {
        "Не удалось связаться с погодным сервисом. Проверьте подключение к интернету."
    } else {
        "Не удалось получить прогноз от погодного сервиса. Попробуйте позже."
    }
}

async fn request_weather<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: reqwest::Url) -> Result<T, String> {
    for attempt in 0..2 {
        match client.get(url.clone()).send().await.and_then(|response| response.error_for_status()) {
            Ok(response) => return response.json().await.map_err(|error| {
                log::warn!("Weather response could not be decoded: {error}");
                "Погодный сервис прислал некорректные данные. Попробуйте позже.".into()
            }),
            Err(error) => {
                log::warn!("Weather request attempt {} failed: {error:?}", attempt + 1);
                let retryable = error.is_timeout() || error.is_connect()
                    || error.status().is_some_and(|status| status.is_server_error() || status.as_u16() == 429);
                if attempt == 1 || !retryable { return Err(weather_error(&error).into()); }
                // Sleep off the UI/async executor thread while backing off transient failures.
                let delay = Duration::from_millis(500 * (attempt + 1));
                let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(delay)).await;
            }
        }
    }
    unreachable!()
}

#[derive(Debug, Serialize)]
pub struct WeekWeather {
    seasonal: bool,
    city: String,
    requested_city: String,
    region: String,
    timezone: String,
    updated_at: String,
    days: Vec<WeatherDay>,
}

#[derive(Debug, Serialize)]
pub struct WeatherDay {
    date: String,
    code: u16,
    high: f64,
    low: f64,
    rain: Option<f64>,
    wind: Option<f64>,
    uv: Option<f64>,
}

#[derive(Deserialize)]
struct Locations { #[serde(default)] results: Vec<Location> }
#[derive(Deserialize)]
struct Location {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)] admin1: String,
}
#[derive(Deserialize)]
struct Forecast { timezone: String, daily: Daily }
#[derive(Deserialize)]
struct Daily {
    time: Vec<String>,
    #[serde(default)] weather_code: Vec<Option<u16>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)] precipitation_probability_max: Vec<Option<f64>>,
    #[serde(default)] wind_speed_10m_max: Vec<Option<f64>>,
    #[serde(default)] uv_index_max: Vec<Option<f64>>,
}

#[tauri::command]
pub fn center_get_weather_city() -> String {
    jarvis_core::weather_city::get()
}

#[tauri::command]
pub async fn center_get_weather(city: Option<String>, days: Option<usize>) -> Result<WeekWeather, String> {
    let length=days.unwrap_or(7);
    if ![7,30,60].contains(&length) {return Err("Выберите неделю, 30 или 60 дней.".into());}
    let seasonal=length>7;
    let explicit_city = city.as_ref().is_some_and(|value| !value.trim().is_empty());
    let city = city.filter(|value| !value.trim().is_empty()).unwrap_or_else(jarvis_core::weather_city::get);
    let city = city.trim();
    if city.chars().count() > 100 { return Err("Название города слишком длинное".into()); }
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(12)).timeout(Duration::from_secs(25)).build()
        .map_err(|_| "Не удалось подключиться к погодному сервису. Попробуйте позже.".to_string())?;
    let mut geo_url = reqwest::Url::parse("https://geocoding-api.open-meteo.com/v1/search").map_err(|e| e.to_string())?;
    geo_url.query_pairs_mut().extend_pairs([("name", city), ("count", "1"), ("language", "ru"), ("format", "json")]);
    let locations: Locations = request_weather(&client, geo_url).await?;
    let location = locations.results.into_iter().next()
        .ok_or_else(|| format!("Город «{city}» не найден. Уточните название или область."))?;
    let mut forecast_url = reqwest::Url::parse(if seasonal {"https://seasonal-api.open-meteo.com/v1/seasonal"} else {"https://api.open-meteo.com/v1/forecast"}).map_err(|e| e.to_string())?;
    forecast_url.query_pairs_mut().extend_pairs([
            ("latitude", location.latitude.to_string()),
            ("longitude", location.longitude.to_string()),
            ("daily", if seasonal {"temperature_2m_max,temperature_2m_min"} else {"weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,wind_speed_10m_max,uv_index_max"}.into()),
            ("wind_speed_unit", "ms".into()),
            ("timezone", "auto".into()),
            ("forecast_days", length.to_string()),
        ]);
    if seasonal { forecast_url.query_pairs_mut().append_pair("models","ecmwf_seasonal_ensemble_mean_seamless"); }
    let forecast: Forecast = request_weather(&client, forecast_url).await?;
    let days = parse_forecast_days(forecast.daily,length,seasonal)?;
    if explicit_city { jarvis_core::weather_city::set(city)?; }
    Ok(WeekWeather {
        seasonal,
        city: location.name, requested_city: city.to_owned(), region: location.admin1, timezone: forecast.timezone,
        updated_at: chrono::Utc::now().to_rfc3339(), days,
    })
}

fn parse_days(daily: Daily) -> Result<Vec<WeatherDay>, String> {
    parse_forecast_days(daily,7,false)
}
fn parse_forecast_days(daily: Daily, length: usize, seasonal: bool) -> Result<Vec<WeatherDay>, String> {
    let optional = |values: &[Option<f64>], index: usize| values.get(index).copied().flatten().filter(|v| v.is_finite());
    let mut days = Vec::with_capacity(length);
    for i in 0..length {
        let date = daily.time.get(i).filter(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok());
        let code = if seasonal {Some(999)} else {daily.weather_code.get(i).copied().flatten()};
        let high = optional(&daily.temperature_2m_max, i);
        let low = optional(&daily.temperature_2m_min, i);
        let (Some(date), Some(code), Some(high), Some(low)) = (date, code, high, low) else {
            return Err("Погодный сервис вернул неполный прогноз на выбранный период. Попробуйте обновить.".into());
        };
        if low > high { return Err("Погодный сервис вернул противоречивые температуры".into()); }
        days.push(WeatherDay {
            date: date.clone(), code, high, low,
            rain: optional(&daily.precipitation_probability_max, i).filter(|v| (0.0..=100.0).contains(v)),
            wind: optional(&daily.wind_speed_10m_max, i).filter(|v| *v >= 0.0),
            uv: optional(&daily.uv_index_max, i).filter(|v| *v >= 0.0),
        });
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seasonal_forecast_has_no_invented_conditions() {
        let daily:Daily=serde_json::from_value(serde_json::json!({"time":["2026-10-03","2026-10-04"],"temperature_2m_max":[12.0,11.0],"temperature_2m_min":[5.0,4.0]})).unwrap();
        let days=parse_forecast_days(daily,2,true).unwrap();
        assert_eq!(days.len(),2);assert_eq!(days[0].code,999);assert!(days[0].rain.is_none());
    }
    #[test]
    #[ignore="requires live network access"]
    fn live_weather_periods() {
        for length in [7,30,60] {
            let result=tauri::async_runtime::block_on(center_get_weather(None,Some(length))).unwrap();
            assert_eq!(result.days.len(),length);assert_eq!(result.seasonal,length>7);
        }
    }

    #[test]
    fn incomplete_forecast_is_not_presented_as_a_week() {
        let daily: Daily = serde_json::from_value(serde_json::json!({
            "time": ["2026-09-30"], "weather_code": [3],
            "temperature_2m_max": [15.0], "temperature_2m_min": [5.0],
            "precipitation_probability_max": [null], "wind_speed_10m_max": [null], "uv_index_max": [null]
        })).unwrap();
        assert!(parse_days(daily).unwrap_err().contains("неполный"));
    }
}
