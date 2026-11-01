use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{PhysicalPosition, Window};

static ANIMATING: AtomicBool = AtomicBool::new(false);
static ENTRANCE_REQUESTED: AtomicBool = AtomicBool::new(false);
const DURATION: Duration = Duration::from_millis(720);
const FRAME: Duration = Duration::from_millis(16);
/// Short rise instead of a full-screen slide: moving a window far is visibly jerky.
const RISE_PX: f64 = 110.0;

fn smootherstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Fast start, long soft landing (same feel as the UI's cubic-bezier(.22,1,.36,1)).
fn ease_out_quint(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(5)
}

/// Window opacity via a layered window, plus vsync-paced frames from DWM.
#[cfg(windows)]
mod native {
    use tauri::Window;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::DwmFlush;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED,
    };

    pub struct Fade(HWND);

    impl Fade {
        pub fn begin(window: &Window) -> Option<Self> {
            let hwnd = window.hwnd().ok()?.0 as HWND;
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
                if SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA) == 0 { return None; }
            }
            Some(Self(hwnd))
        }
        pub fn set(&self, opacity: f64) {
            unsafe { SetLayeredWindowAttributes(self.0, 0, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8, LWA_ALPHA); }
        }
    }

    impl Drop for Fade {
        // Always end fully opaque and non-layered, even if the animation fails midway.
        fn drop(&mut self) {
            unsafe {
                SetLayeredWindowAttributes(self.0, 0, 255, LWA_ALPHA);
                let style = GetWindowLongPtrW(self.0, GWL_EXSTYLE);
                SetWindowLongPtrW(self.0, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
            }
        }
    }

    /// Waits for the next composition frame; false when DWM is unavailable.
    pub fn wait_frame() -> bool { unsafe { DwmFlush() >= 0 } }
}

#[cfg(not(windows))]
mod native {
    use tauri::Window;
    pub struct Fade;
    impl Fade {
        pub fn begin(_: &Window) -> Option<Self> { None }
        pub fn set(&self, _: f64) {}
    }
    pub fn wait_frame() -> bool { false }
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

    // Start slightly below the centre and fully transparent, then rise and fade in together.
    let rise = (RISE_PX * window.scale_factor().unwrap_or(1.0)).round() as i32;
    let start_y = target_y + rise;
    let fade = native::Fade::begin(window);
    window.set_position(PhysicalPosition::new(target_x, start_y)).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_position(PhysicalPosition::new(target_x, start_y)).map_err(|e| e.to_string())?;

    let actual_start = window.outer_position().map_err(|e| e.to_string())?.y;
    log::info!("Window entrance: start_y={actual_start}, target_y={target_y}, fade={}", fade.is_some());

    let started = Instant::now();
    let mut frames = 0_u32;
    loop {
        let progress = (started.elapsed().as_secs_f64() / DURATION.as_secs_f64()).min(1.0);
        let y = actual_start as f64 + (target_y - actual_start) as f64 * ease_out_quint(progress);
        window.set_position(PhysicalPosition::new(target_x, y.round() as i32)).map_err(|e| e.to_string())?;
        // Opacity finishes first so the window is solid while it settles.
        if let Some(fade) = &fade { fade.set(smootherstep(progress / 0.7)); }
        frames += 1;
        if progress >= 1.0 { break; }
        if !native::wait_frame() {
            let next_frame = started + FRAME * frames;
            if let Some(remaining) = next_frame.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
        }
    }
    window.set_position(PhysicalPosition::new(target_x, target_y)).map_err(|e| e.to_string())?;
    drop(fade);
    log::info!("Window entrance finished at y={} in {:?} ({frames} frames)", window.outer_position().map_err(|e| e.to_string())?.y, started.elapsed());
    // Repeat after the initial ShowWindow: SW_HIDE from the launcher must not leave
    // a running application invisible, even though the entrance was requested.
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ease_out_quint, smootherstep};
    #[test]
    fn entrance_easing_has_soft_start_and_finish() {
        assert_eq!(smootherstep(0.0), 0.0);
        assert_eq!(smootherstep(1.0), 1.0);
        assert!((smootherstep(0.5) - 0.5).abs() < 1e-9);
        assert!(smootherstep(0.1) < 0.1);
        assert!(smootherstep(0.9) > 0.9);
    }
    #[test]
    fn rise_moves_fast_first_and_lands_softly() {
        assert_eq!(ease_out_quint(0.0), 0.0);
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert!(ease_out_quint(0.3) > 0.8);
        assert!(ease_out_quint(0.9) > 0.9999);
    }
}
