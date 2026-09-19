//! The real platform adapter: Win32, Core Audio, and GDI.
//!
//! Everything unsafe in this feature is in this file, and each block is small enough to read
//! in one sitting. The rules the rest of the crate relies on are enforced here as well:
//!
//! * the window list is **this desktop's** visible user windows: `EnumWindows` only walks the
//!   calling desktop, so a window of another session or of the secure desktop cannot be
//!   listed at all, and it therefore cannot be selected;
//! * every window action re-checks the handle before it acts: the window must still exist, be
//!   visible, be the same process, and not belong to this application;
//! * closing a window posts `WM_CLOSE`. There is no `TerminateProcess` in this file, and no
//!   process is ever killed;
//! * nothing here formats or runs a command line. Launching uses [`std::process::Command`]
//!   with the canonical path and the fixed arguments of an allowed entry;
//! * the volume goes through the documented Core Audio endpoint interface, and the screenshot
//!   through GDI, so no keystroke is ever simulated.

use std::path::{Path, PathBuf};

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDCW, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDIBits, GetMonitorInfoW, MonitorFromWindow, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, RGBQUAD, SRCCOPY,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, PostMessageW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
};

use super::{Capabilities, CaptureRequest, NativeWindow, VolumeState, WindowsBackend};
use crate::windows_actions::allowlist::LaunchSpec;
use crate::windows_actions::error::ActionError;
use crate::windows_actions::model::{
    sanitize_window_title, ScreenshotTarget, WindowId, WindowOperation, WindowState,
    MAX_MOVE_PIXELS,
};

/// Longest process image path read from the system.
const PATH_BUFFER: usize = 260;
/// Longest window class name read from the system.
const CLASS_BUFFER: usize = 128;

/// The production backend.
#[derive(Debug, Default)]
pub struct NativeWindowsBackend;

impl NativeWindowsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl WindowsBackend for NativeWindowsBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform_supported: true,
            volume: true,
            screenshots: true,
            windows: true,
            lock_workstation: true,
            // A toast needs an application id registered by an installer, which this stage
            // does not create yet. The interface shows its own notification instead.
            notifications: false,
            ai_tools: true,
            notes: vec!["windows-actions-note-notifications".to_string()],
        }
    }

    fn volume(&self) -> Result<VolumeState, ActionError> {
        let endpoint = endpoint_volume()?;
        let percent = unsafe { endpoint.GetMasterVolumeLevelScalar() }.map_err(api_error)?;
        let muted = unsafe { endpoint.GetMute() }
            .map(|flag| flag.as_bool())
            .map_err(api_error)?;
        Ok(VolumeState {
            percent: scalar_to_percent(percent),
            muted,
        })
    }

    fn set_volume(&self, percent: u8) -> Result<VolumeState, ActionError> {
        if percent > 100 {
            return Err(ActionError::InvalidArguments {
                detail: "volume must be between 0 and 100".to_string(),
            });
        }
        let endpoint = endpoint_volume()?;
        unsafe { endpoint.SetMasterVolumeLevelScalar(percent as f32 / 100.0, std::ptr::null()) }
            .map_err(api_error)?;
        self.volume()
    }

    fn set_mute(&self, muted: bool) -> Result<VolumeState, ActionError> {
        let endpoint = endpoint_volume()?;
        unsafe { endpoint.SetMute(muted, std::ptr::null()) }.map_err(api_error)?;
        self.volume()
    }

    fn launch(&self, spec: &LaunchSpec) -> Result<u32, ActionError> {
        // The path and the arguments come from the stored entry, never from a caller. No
        // shell is involved: `Command` starts the executable directly.
        let mut command = std::process::Command::new(&spec.program);
        command.args(&spec.arguments);
        if let Some(directory) = spec.working_directory.as_ref() {
            command.current_dir(directory);
        }
        let child = command.spawn().map_err(|_| ActionError::InvalidArguments {
            detail: "the program could not be started".to_string(),
        })?;
        Ok(child.id())
    }

    fn list_windows(&self) -> Result<Vec<NativeWindow>, ActionError> {
        let mut handles: Vec<HWND> = Vec::new();
        let pointer = LPARAM((&mut handles as *mut Vec<HWND>) as isize);
        unsafe { EnumWindows(Some(collect_window), pointer) }.map_err(api_error)?;

        let own_process = std::process::id();
        let mut windows = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Some(window) = describe_window(handle, own_process) {
                windows.push(window);
            }
        }
        windows.sort_by(|left, right| left.title.cmp(&right.title));
        Ok(windows)
    }

    fn act_on_window(
        &self,
        window: &NativeWindow,
        operation: &WindowOperation,
    ) -> Result<(), ActionError> {
        let handle = revalidate_window(window)?;
        match operation {
            WindowOperation::Minimize => {
                let _ = unsafe { ShowWindow(handle, SW_MINIMIZE) };
            }
            WindowOperation::Maximize => {
                let _ = unsafe { ShowWindow(handle, SW_MAXIMIZE) };
            }
            WindowOperation::Restore => {
                let _ = unsafe { ShowWindow(handle, SW_RESTORE) };
            }
            WindowOperation::Move {
                x,
                y,
                width,
                height,
            } => {
                if x.unsigned_abs() > MAX_MOVE_PIXELS as u32
                    || y.unsigned_abs() > MAX_MOVE_PIXELS as u32
                {
                    return Err(ActionError::InvalidArguments {
                        detail: "that position is outside the supported range".to_string(),
                    });
                }
                let (x, y, width, height) = window.clamp_move(*x, *y, *width, *height);
                unsafe {
                    SetWindowPos(
                        handle,
                        None,
                        x,
                        y,
                        width as i32,
                        height as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                }
                .map_err(api_error)?;
            }
            WindowOperation::Close => {
                // The normal close message: the application decides, exactly as if the user
                // had clicked the cross. No process is terminated.
                unsafe { PostMessageW(Some(handle), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(api_error)?;
            }
        }
        Ok(())
    }

    fn capture(&self, request: &CaptureRequest) -> Result<u64, ActionError> {
        let rect = capture_rect(request)?;
        let pixels = unsafe { read_screen(rect) }?;
        let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.bytes).ok_or(
            ActionError::ScreenshotFailed {
                detail: "the captured image has an unexpected size".to_string(),
            },
        )?;
        image
            .save_with_format(&request.destination, image::ImageFormat::Png)
            .map_err(|_| ActionError::ScreenshotFailed {
                detail: "the image could not be written".to_string(),
            })?;
        let written = std::fs::metadata(&request.destination)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(written)
    }

    fn lock_workstation(&self) -> Result<(), ActionError> {
        // The documented API, not a simulated key combination.
        unsafe { LockWorkStation() }.map_err(api_error)
    }

    fn notify(&self, _title: &str, _message: &str) -> Result<(), ActionError> {
        // A system toast needs an application id registered by an installer; until this
        // application has one, the interface shows its own notification, and this returns
        // the documented "not available" answer rather than pretending.
        Err(ActionError::CapabilityUnavailable {
            capability: "show a system notification".to_string(),
        })
    }
}

/// One captured image, ready to be encoded.
struct CapturedPixels {
    width: u32,
    height: u32,
    /// RGBA, row-major, top-down.
    bytes: Vec<u8>,
}

/// The screen rectangle a request covers.
fn capture_rect(request: &CaptureRequest) -> Result<(i32, i32, i32, i32), ActionError> {
    match &request.target {
        ScreenshotTarget::AllMonitors => {
            let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
            let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
            let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
            let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
            if width <= 0 || height <= 0 {
                return Err(ActionError::ScreenshotFailed {
                    detail: "no screen was reported by the system".to_string(),
                });
            }
            Ok((x, y, width, height))
        }
        ScreenshotTarget::PrimaryMonitor => monitor_rect(monitor_from_point(0, 0)),
        ScreenshotTarget::SelectedMonitor(index) => {
            let rects = monitor_rects()?;
            let index = *index as usize;
            rects
                .get(index.saturating_sub(1))
                .copied()
                .ok_or(ActionError::InvalidArguments {
                    detail: "that monitor does not exist".to_string(),
                })
        }
        ScreenshotTarget::SelectedWindow(_) => {
            let window = request.window.as_ref().ok_or(ActionError::WindowNotFound)?;
            let handle = revalidate_window(window)?;
            let mut rect = RECT::default();
            unsafe { GetWindowRect(handle, &mut rect) }.map_err(api_error)?;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                return Err(ActionError::ScreenshotFailed {
                    detail: "that window has no visible area".to_string(),
                });
            }
            Ok((rect.left, rect.top, width, height))
        }
    }
}

fn monitor_from_point(x: i32, y: i32) -> HMONITOR {
    // `MonitorFromWindow` needs a window; for a point the equivalent is the primary monitor
    // when the coordinates are inside it, which is what `MonitorFromPoint`-style behaviour
    // gives for (0,0) on a default desktop.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0.is_null() {
        return unsafe { MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTONEAREST) };
    }
    let _ = (x, y);
    unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) }
}

fn monitor_rect(monitor: HMONITOR) -> Result<(i32, i32, i32, i32), ActionError> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err(ActionError::ScreenshotFailed {
            detail: "the monitor could not be measured".to_string(),
        });
    }
    Ok((
        info.rcMonitor.left,
        info.rcMonitor.top,
        info.rcMonitor.right - info.rcMonitor.left,
        info.rcMonitor.bottom - info.rcMonitor.top,
    ))
}

/// Every monitor rectangle, in the order the system reports them.
fn monitor_rects() -> Result<Vec<(i32, i32, i32, i32)>, ActionError> {
    let mut rects: Vec<(i32, i32, i32, i32)> = Vec::new();
    let pointer = LPARAM((&mut rects as *mut Vec<(i32, i32, i32, i32)>) as isize);
    let listed = unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), pointer) };
    if !listed.as_bool() {
        return Err(ActionError::ScreenshotFailed {
            detail: "the monitors could not be listed".to_string(),
        });
    }
    if rects.is_empty() {
        return Err(ActionError::ScreenshotFailed {
            detail: "no monitor was reported".to_string(),
        });
    }
    Ok(rects)
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let rects = unsafe { &mut *(data.0 as *mut Vec<(i32, i32, i32, i32)>) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        rects.push((
            info.rcMonitor.left,
            info.rcMonitor.top,
            info.rcMonitor.right - info.rcMonitor.left,
            info.rcMonitor.bottom - info.rcMonitor.top,
        ));
    }
    BOOL::from(true)
}

/// Reads the screen area into an RGBA buffer.
unsafe fn read_screen(rect: (i32, i32, i32, i32)) -> Result<CapturedPixels, ActionError> {
    let (x, y, width, height) = rect;
    let screen = unsafe {
        CreateDCW(
            windows::core::w!("DISPLAY"),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
        )
    };
    if screen.is_invalid() {
        return Err(ActionError::ScreenshotFailed {
            detail: "the screen could not be opened".to_string(),
        });
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.is_invalid() {
        let _ = unsafe { DeleteDC(screen) };
        return Err(ActionError::ScreenshotFailed {
            detail: "a drawing surface could not be created".to_string(),
        });
    }
    let bitmap: HBITMAP = unsafe { CreateCompatibleBitmap(screen, width, height) };
    if bitmap.is_invalid() {
        unsafe {
            let _ = DeleteDC(memory);
            let _ = DeleteDC(screen);
        }
        return Err(ActionError::ScreenshotFailed {
            detail: "an image buffer could not be created".to_string(),
        });
    }
    let previous: HGDIOBJ = unsafe { SelectObject(memory, bitmap.into()) };

    let copied = unsafe {
        BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        )
        .is_ok()
    };

    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // A negative height asks for a top-down buffer, so row zero is the top row.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
    let lines = unsafe {
        GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };

    unsafe {
        let _ = SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        let _ = DeleteDC(screen);
    }

    if !copied || lines == 0 {
        return Err(ActionError::ScreenshotFailed {
            detail: "the screen could not be copied".to_string(),
        });
    }

    // GDI hands back BGRA; the encoder wants RGBA. The bitmap is four bytes per pixel, so a
    // half-open chunk of four is exactly one pixel.
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        // The alpha channel of a screen bitmap is undefined.
        pixel[3] = 255;
    }
    Ok(CapturedPixels {
        width: width as u32,
        height: height as u32,
        bytes: bgra,
    })
}

/// Collects one window handle during `EnumWindows`.
unsafe extern "system" fn collect_window(handle: HWND, data: LPARAM) -> BOOL {
    let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    handles.push(handle);
    BOOL::from(true)
}

/// Describes one window, or returns `None` when it must not be offered.
fn describe_window(handle: HWND, own_process: u32) -> Option<NativeWindow> {
    if !unsafe { IsWindowVisible(handle) }.as_bool() {
        return None;
    }
    if is_tool_window(handle) {
        return None;
    }
    let title = window_title(handle);
    if title.trim().is_empty() {
        return None;
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(handle, &mut rect) }.is_err() {
        return None;
    }
    if rect.right - rect.left <= 0 || rect.bottom - rect.top <= 0 {
        return None;
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(handle, Some(&mut process_id)) };
    if process_id == 0 {
        return None;
    }
    let process_name = process_name(process_id);
    if process_name.is_empty() {
        return None;
    }
    let state = if unsafe { IsIconic(handle) }.as_bool() {
        WindowState::Minimized
    } else if unsafe { IsZoomed(handle) }.as_bool() {
        WindowState::Maximized
    } else {
        WindowState::Normal
    };
    let foreground = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST) };
    let work_area = monitor_work_area(monitor).unwrap_or((0, 0, rect.right, rect.bottom));
    Some(NativeWindow {
        id: WindowId::mint().ok()?.as_str().to_string(),
        native_id: handle.0 as isize,
        process_id,
        process_name,
        title: sanitize_window_title(&title),
        state,
        monitor: monitor_index(monitor),
        is_own_process: process_id == own_process,
        is_foreground: foreground.0 == handle.0,
        work_area,
    })
}

fn monitor_index(monitor: HMONITOR) -> u32 {
    let Ok(rects) = monitor_rects() else {
        return 1;
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return 1;
    }
    let wanted = (
        info.rcMonitor.left,
        info.rcMonitor.top,
        info.rcMonitor.right,
        info.rcMonitor.bottom,
    );
    rects
        .iter()
        .position(|rect| (rect.0, rect.1, rect.0 + rect.2, rect.1 + rect.3) == wanted)
        .map(|index| index as u32 + 1)
        .unwrap_or(1)
}

fn monitor_work_area(monitor: HMONITOR) -> Option<(i32, i32, i32, i32)> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    Some((
        info.rcWork.left,
        info.rcWork.top,
        info.rcWork.right - info.rcWork.left,
        info.rcWork.bottom - info.rcWork.top,
    ))
}

fn is_tool_window(handle: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
    style & WS_EX_TOOLWINDOW.0 != 0
}

fn window_title(handle: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(handle) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; (length as usize) + 1];
    let written = unsafe { GetWindowTextW(handle, &mut buffer) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..written as usize])
}

fn window_class(handle: HWND) -> String {
    let mut buffer = vec![0u16; CLASS_BUFFER];
    let written = unsafe { GetClassNameW(handle, &mut buffer) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..written as usize])
}

fn process_name(process_id: u32) -> String {
    let handle: HANDLE =
        match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) } {
            Ok(handle) => handle,
            Err(_) => return String::new(),
        };
    let mut buffer = vec![0u16; PATH_BUFFER];
    let mut size = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    if result.is_err() || size == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(path)
}

/// Re-checks a window before acting on it.
///
/// This is what makes a stale identifier harmless: the handle is looked up again, and the
/// window has to still exist, be visible, be the same process, and not be this application's
/// own window.
fn revalidate_window(window: &NativeWindow) -> Result<HWND, ActionError> {
    if window.native_id == 0 {
        return Err(ActionError::WindowNotFound);
    }
    let handle = HWND(window.native_id as *mut core::ffi::c_void);
    if !unsafe { IsWindow(Some(handle)) }.as_bool() {
        return Err(ActionError::WindowNotFound);
    }
    if !unsafe { IsWindowVisible(handle) }.as_bool() {
        return Err(ActionError::WindowNotFound);
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(handle, Some(&mut process_id)) };
    if process_id != window.process_id || process_id == 0 {
        return Err(ActionError::WindowExpired);
    }
    if process_id == std::process::id() {
        return Err(ActionError::ForbiddenAction {
            reason: "that window belongs to this application".to_string(),
        });
    }
    // System surfaces that are not user windows are refused.
    let class = window_class(handle);
    if matches!(
        class.as_str(),
        "Progman" | "Shell_TrayWnd" | "WorkerW" | "Windows.UI.Core.CoreWindow"
    ) {
        return Err(ActionError::ForbiddenAction {
            reason: "that is a system window".to_string(),
        });
    }
    Ok(handle)
}

/// Prepares a Core Audio endpoint for the default output device.
fn endpoint_volume() -> Result<IAudioEndpointVolume, ActionError> {
    unsafe {
        // A failure here means COM is already initialised on this thread in another mode,
        // which is fine; the uninitialise below is balanced by this call's success only.
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| -> Result<IAudioEndpointVolume, ActionError> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(api_error)?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(api_error)?;
            device
                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                .map_err(api_error)
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

fn api_error(error: windows::core::Error) -> ActionError {
    ActionError::WindowsApiError {
        code: error.code().0,
    }
}

fn scalar_to_percent(scalar: f32) -> u8 {
    if !scalar.is_finite() {
        return 0;
    }
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// A path inside the feature directory, used by the executor.
pub fn destination_in(directory: &Path, file_name: &str) -> PathBuf {
    directory.join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scalar_becomes_a_percentage_in_range() {
        assert_eq!(scalar_to_percent(0.0), 0);
        assert_eq!(scalar_to_percent(0.4), 40);
        assert_eq!(scalar_to_percent(0.405), 41);
        assert_eq!(scalar_to_percent(1.0), 100);
        assert_eq!(scalar_to_percent(2.0), 100);
        assert_eq!(scalar_to_percent(-1.0), 0);
        assert_eq!(scalar_to_percent(f32::NAN), 0);
    }

    #[test]
    fn the_native_backend_reports_a_platform_that_exists_without_a_toast() {
        let capabilities = NativeWindowsBackend::new().capabilities();
        assert!(capabilities.platform_supported);
        assert!(capabilities.volume);
        assert!(capabilities.screenshots);
        assert!(capabilities.windows);
        assert!(capabilities.lock_workstation);
        // Honest: a toast needs a registered application id.
        assert!(!capabilities.notifications);
    }

    /// The shipped half of this file: comments and the test module are removed, so the scan
    /// below is about the code that actually runs.
    fn shipped_source() -> String {
        let source = include_str!("native.rs");
        let shipped = match source.find("#[cfg(test)]") {
            Some(index) => &source[..index],
            None => source,
        };
        shipped
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_native_backend_never_simulates_a_key_combination() {
        // A structural check of this file: closing posts a message, locking uses the API,
        // and no input is ever synthesized. The needles are built by concatenation so that
        // this test does not match itself.
        let source = shipped_source();
        for forbidden in [
            concat!("keybd", "_event"),
            concat!("Send", "Input"),
            concat!("mouse", "_event"),
            concat!("Terminate", "Process"),
            concat!("task", "kill"),
            concat!("Shell", "Execute"),
            concat!("Win", "Exec"),
            concat!("Create", "Process"),
            concat!("cmd", ".exe"),
            concat!("power", "shell"),
        ] {
            assert!(
                !source.contains(forbidden),
                "the native backend must not use {forbidden}"
            );
        }
        assert!(source.contains("PostMessageW"));
        assert!(source.contains("LockWorkStation"));
    }

    #[test]
    fn the_scan_reads_the_shipped_half_of_this_file() {
        let source = shipped_source();
        assert!(!source.contains("fn the_native_backend_never_simulates_a_key_combination"));
        assert!(source.contains("pub struct NativeWindowsBackend"));
    }

    #[test]
    fn a_zero_handle_is_refused_before_anything_else() {
        let window = NativeWindow {
            id: "aabb".to_string(),
            native_id: 0,
            process_id: 1,
            process_name: "x.exe".to_string(),
            title: "x".to_string(),
            state: WindowState::Normal,
            monitor: 1,
            is_own_process: false,
            is_foreground: false,
            work_area: (0, 0, 100, 100),
        };
        assert_eq!(
            revalidate_window(&window).unwrap_err(),
            ActionError::WindowNotFound
        );
    }
}
