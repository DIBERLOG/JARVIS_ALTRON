import { invoke } from "@tauri-apps/api/core"
import { writable } from 'svelte/store'
export const weatherPeriod = writable(7)

export type WeatherDay = { date: string; code: number; high: number; low: number; rain: number | null; wind: number | null; uv: number | null }
export type WeekWeather = { city: string; requested_city: string; region: string; timezone: string; updated_at: string; days: WeatherDay[]; hours?:{time:string;temperature:number;rain:number|null}[]; seasonal?: boolean; warning?: string }
export type WeatherKind = "sun" | "partly" | "cloud" | "fog" | "rain" | "snow" | "storm" | "unknown"

let cached: WeekWeather | null = null
export const getWeatherCity = () => invoke<string>("center_get_weather_city")
export async function getWeekWeather(city?: string, force = false, days = 7): Promise<WeekWeather> {
    const requested = city || await getWeatherCity()
    if (!force && cached && cached.days.length === days && Date.now() - Date.parse(cached.updated_at) < 10 * 60_000 && requested.toLowerCase() === cached.requested_city.toLowerCase()) return cached
    try {
        cached = await invoke<WeekWeather>("center_get_weather", { city: city || null, days })
        return cached
    } catch (error) {
        // Keep actual previously received data, never substitute another city's forecast.
        if (cached && cached.days.length === days && requested.toLowerCase() === cached.requested_city.toLowerCase()
            && Date.now() - Date.parse(cached.updated_at) < 3 * 60 * 60_000) {
            return { ...cached, warning: String(error) }
        }
        throw error
    }
}

export function weatherKind(code: number): WeatherKind {
    if (code === 0) return "sun"
    if (code === 1 || code === 2) return "partly"
    if (code === 3) return "cloud"
    if (code === 45 || code === 48) return "fog"
    if ([51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 80, 81, 82].includes(code)) return "rain"
    if ([71, 73, 75, 77, 85, 86].includes(code)) return "snow"
    if ([95, 96, 99].includes(code)) return "storm"
    return "unknown"
}

export function weatherLabel(code: number): string {
    const kind = weatherKind(code)
    return ({ sun: "Ясно", partly: "Переменная облачность", cloud: "Облачно", fog: "Туман", rain: "Дождь", snow: "Снег", storm: "Гроза", unknown: "Нет описания" })[kind]
}

export function temperature(value: number) {
    const rounded = Math.round(value)
    return `${rounded > 0 ? "+" : ""}${rounded}°`
}

export function clothingFor(day: WeatherDay): { icon: string; text: string }[] {
    const tips = []
    const kind = weatherKind(day.code)
    if (day.high <= 0) tips.push({ icon: "coat", text: "Пуховик, шапка и перчатки" })
    else if (day.high < 10) tips.push({ icon: "coat", text: "Тёплая куртка и свитер" })
    else if (day.high < 18) tips.push({ icon: "coat", text: "Лёгкая куртка и лонгслив" })
    else if (day.high < 25) tips.push({ icon: "shirt", text: "Футболка; утром — лёгкая куртка" })
    else tips.push({ icon: "shirt", text: "Лёгкая одежда из хлопка" })
    if (day.low < 8 && day.high >= 18) tips[0] = { icon: "coat", text: "Футболка днём, тёплая куртка утром" }
    else if (day.low < 8 && day.high >= 10) tips[0] = { icon: "coat", text: "Куртка и лонгслив; утром — свитер" }
    if (["rain", "storm"].includes(kind) || (day.rain !== null && day.rain >= 40)) tips.push({ icon: "umbrella", text: "Зонт и непромокаемая обувь" })
    else if (kind === "snow") tips.push({ icon: "shoe", text: "Утеплённая нескользкая обувь" })
    else tips.push({ icon: "shoe", text: "Удобная закрытая обувь" })
    if (day.wind !== null && day.wind >= 7) tips.push({ icon: "wind", text: "Ветрозащитный верхний слой" })
    else if (day.uv !== null && day.uv >= 3) tips.push({ icon: "sun", text: "Солнцезащитные очки" })
    return tips
}

export function chartGeometry(days: WeatherDay[]) {
    const bottom = Math.floor(Math.min(...days.map(day => day.low)) / 5) * 5 - 2
    const top = Math.ceil(Math.max(...days.map(day => day.high)) / 5) * 5 + 3
    const x = (index: number) => 42 + (index + .5) * (646 / days.length)
    const y = (value: number) => 30 + (top - value) / (top - bottom) * 150
    const points = days.map((day, index) => ({ x: x(index), high: y(day.high), low: y(day.low) }))
    const curve = (key: "high" | "low") => points.reduce((path, point, index) => {
        if (!index) return `M ${point.x} ${point[key]}`
        const previous = points[index - 1], middle = (previous.x + point.x) / 2
        return `${path} C ${middle} ${previous[key]}, ${middle} ${point[key]}, ${point.x} ${point[key]}`
    }, "")
    const highPath = curve("high")
    return { points, highPath, lowPath: curve("low"), areaPath: `${highPath} L ${points[points.length-1].x} 194 L ${points[0].x} 194 Z`, ticks: Array.from({ length: 5 }, (_, index) => { const value = bottom + (top - bottom) * index / 4; return { value: Math.round(value), y: y(value) } }) }
}
