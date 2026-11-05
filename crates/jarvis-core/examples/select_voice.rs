// Local repair utility: use the same encrypted settings transaction as the GUI.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode=std::env::args().nth(1).ok_or("Expected silero, xtts or s2")?;
    if !matches!(mode.as_str(),"silero"|"xtts"|"s2") { return Err("Unknown voice mode".into()); }
    jarvis_core::config::init_dirs()?;
    let settings=jarvis_core::db::init();
    settings.write("tts_mode",&mode)?;
    let saved=jarvis_core::db::latest_settings().ok_or("Cannot read saved settings")?;
    if saved.tts_mode!=mode {return Err("Voice setting was not persisted".into());}
    println!("Voice mode saved: {}",saved.tts_mode);
    Ok(())
}
