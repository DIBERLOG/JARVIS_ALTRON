use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{PhysicalPosition, Window};

static ANIMATING: AtomicBool = AtomicBool::new(false);
static ENTRANCE_REQUESTED: AtomicBool = AtomicBool::new(false);
const DURATION: Duration = Duration::from_millis(1700);
const FRAME: Duration = Duration::from_millis(16);

fn smootherstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[tauri::command]
pub fn animate_window_in(window: Window, reduced_motion: bool) -> Result<(), String> {
    ENTRANCE_REQUESTED.store(true, Ordering::SeqCst);
    if ANIMATING.swap(true, Ordering::SeqCst) { return Ok(()); }
    std::thread::spawn(move || {
        if let Err(error) = slide_window(&window, reduced_motion) {
            log::warn!("Window entrance animation failed: {error}");
            let _ = window.center();
            let _ = window.show();
            let _ = window.set_focus();
        }
        ANIMATING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

pub fn entrance_requested() -> bool {
    ENTRANCE_REQUESTED.load(Ordering::SeqCst)
}

fn slide_window(window: &Window, reduced_motion: bool) -> Result<(), String> {
    let visible = window.is_visible().map_err(|e| e.to_string())?;
    let minimized = window.is_minimized().map_err(|e| e.to_string())?;
    if visible && !minimized {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let monitor = window.current_monitor().map_err(|e| e.to_string())?
        .or(window.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("Не удалось определить монитор")?;
    let work = monitor.work_area();
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let target_x = work.position.x + (work.size.width as i32 - size.width as i32) / 2;
    let target_y = work.position.y + (work.size.height as i32 - size.height as i32) / 2;

    if visible { window.hide().map_err(|e| e.to_string())?; }
    if minimized { window.unminimize().map_err(|e| e.to_string())?; }
    if reduced_motion {
        window.set_position(PhysicalPosition::new(target_x, target_y)).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        // Windows can consume the first ShowWindow call using a launcher's SW_HIDE.
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Keep a narrow strip visible: Windows may reposition a fully off-screen window on show.
    let start_y = work.position.y + work.size.height as i32 - 72;
    window.set_position(PhysicalPosition::new(target_x, start_y)).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_position(PhysicalPosition::new(target_x, start_y)).map_err(|e| e.to_string())?;

    let actual_start = window.outer_position().map_err(|e| e.to_string())?.y;
    log::info!("Window entrance: start_y={actual_start}, target_y={target_y}");

    let started = Instant::now();
    let mut frames = 0_u32;
    loop {
        let progress = (started.elapsed().as_secs_f64() / DURATION.as_secs_f64()).min(1.0);
        let y = actual_start as f64 + (target_y - actual_start) as f64 * smootherstep(progress);
        window.set_position(PhysicalPosition::new(target_x, y.round() as i32)).map_err(|e| e.to_string())?;
        frames += 1;
        if progress >= 1.0 { break; }
        let next_frame = started + FRAME * frames;
        if let Some(remaining) = next_frame.checked_duration_since(Instant::now()) {
            std::thread::sleep(remaining);
        }
    }
    window.set_position(PhysicalPosition::new(target_x, target_y)).map_err(|e| e.to_string())?;
    log::info!("Window entrance finished at y={} in {:?} ({frames} frames)", window.outer_position().map_err(|e| e.to_string())?.y, started.elapsed());
    // Repeat after the initial ShowWindow: SW_HIDE from the launcher must not leave
    // a running application invisible, even though the entrance was requested.
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::smootherstep;
    #[test]
    fn entrance_easing_has_soft_start_and_finish() {
        assert_eq!(smootherstep(0.0), 0.0);
        assert_eq!(smootherstep(1.0), 1.0);
        assert!((smootherstep(0.5) - 0.5).abs() < 1e-9);
        assert!(smootherstep(0.1) < 0.1);
        assert!(smootherstep(0.9) > 0.9);
    }
}
