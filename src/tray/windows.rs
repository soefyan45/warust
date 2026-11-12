//! The Windows tray icon (#134). No thread of its own: a hidden top-level
//! window is made on the main thread, so winit's message loop (which
//! dispatches every window of that thread) delivers the icon's callbacks to
//! this window's procedure.

use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, GWLP_USERDATA, GetCursorPos, GetWindowLongPtrW, HICON, MF_STRING, PostMessageW,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetWindowLongPtrW, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use super::{Event, Handler, Icon, Labels};

/// The message the shell sends to our window for mouse events on the icon.
const CALLBACK_MESSAGE: u32 = WM_APP + 1;
/// Menu command ids (zero means "nothing picked" for `TPM_RETURNCMD`).
const ID_OPEN: usize = 1;
const ID_QUIT: usize = 2;
/// Length of `NOTIFYICONDATAW::szTip`, with its NUL.
const TIP_LEN: usize = 128;

/// What the window procedure needs. Boxed so its address stays put; the
/// window keeps the pointer in `GWLP_USERDATA`. Never changed once the window
/// can see it, so the procedure only ever takes shared references.
struct State {
    handler: Handler,
    open: Vec<u16>,
    quit: Vec<u16>,
    data: NOTIFYICONDATAW,
    /// Id of the broadcast the shell sends when the taskbar is recreated.
    taskbar_created: u32,
}

/// The icon while it is shown.
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Owned here, freed in `drop` after the window is gone.
    state: *mut State,
}

impl Tray {
    /// `None` when the icon can't be added.
    pub fn new(icon: &Icon, labels: &Labels, handler: Handler) -> Option<Self> {
        let icon = make_icon(icon)?;
        let Some(hwnd) = make_window() else {
            // SAFETY: `icon` was just created and nothing else uses it.
            let _ = unsafe { DestroyIcon(icon) };
            return None;
        };

        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: CALLBACK_MESSAGE,
            hIcon: icon,
            ..Default::default()
        };
        copy_tip(&mut data.szTip, &labels.tooltip);

        let state = Box::into_raw(Box::new(State {
            handler,
            open: wide(&labels.open),
            quit: wide(&labels.quit),
            data,
            // SAFETY: plain call with a static NUL-terminated string.
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        }));
        // SAFETY: `hwnd` is our live window on this thread; `state` is a valid
        // box that stays alive until `drop`.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as _) };

        // SAFETY: `state` was just made and is not freed yet; `data` is a fully
        // initialised struct with the right `cbSize`.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &(*state).data) }.as_bool();
        if !added {
            log::warn!("could not add the tray icon");
            // SAFETY: undo the setup in reverse order; the window is destroyed
            // before the state it points to is freed.
            unsafe {
                let _ = DestroyWindow(hwnd);
                drop(Box::from_raw(state));
                let _ = DestroyIcon(icon);
            }
            return None;
        }
        Some(Self { hwnd, icon, state })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: `state` is still ours (freed below). The window goes first so
        // the procedure can no longer see the state, and the icon is destroyed
        // after the shell has let go of it.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &(*self.state).data);
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.icon);
            drop(Box::from_raw(self.state));
        }
    }
}

/// UTF-16 with a trailing NUL, as `AppendMenuW` wants it.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Copies as much of `text` as fits, always leaving the final NUL.
fn copy_tip(dest: &mut [u16; TIP_LEN], text: &str) {
    for (slot, unit) in dest.iter_mut().take(TIP_LEN - 1).zip(text.encode_utf16()) {
        *slot = unit;
    }
}

/// A 32-bit icon from RGBA pixels. `CreateIcon` wants BGRA plus an AND mask;
/// the mask stays zero because the alpha channel does the cutting.
fn make_icon(icon: &Icon) -> Option<HICON> {
    let (width, height) = (icon.width as usize, icon.height as usize);
    if width == 0 || height == 0 || icon.rgba.len() < width * height * 4 {
        return None;
    }
    let bgra: Vec<u8> = icon.rgba[..width * height * 4]
        .chunks_exact(4)
        .flat_map(|px| [px[2], px[1], px[0], px[3]])
        .collect();
    // One bit per pixel, each row padded to a whole number of 16-bit words.
    let mask = vec![0u8; width.div_ceil(16) * 2 * height];
    // SAFETY: both buffers outlive the call (the system copies them) and have
    // the sizes `CreateIcon` reads for these dimensions.
    unsafe { CreateIcon(None, width as i32, height as i32, 1, 32, mask.as_ptr(), bgra.as_ptr()) }.ok()
}

/// The hidden window. A message-only window would not get the broadcast
/// `TaskbarCreated`, so this is a normal top-level one that is never shown.
fn make_window() -> Option<HWND> {
    const CLASS: PCWSTR = w!("warust_tray");
    // SAFETY: plain calls; the class name is static and the procedure matches
    // the required signature.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // Zero also when the class already exists (the tray was made before),
        // which is fine: only a missing class shows up as a window error below.
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("warust"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .inspect_err(|e| log::warn!("could not create the tray window: {e}"))
        .ok()
    }
}

/// Shows the right-click menu at the pointer and reports the chosen item.
fn show_menu(hwnd: HWND, state: &State) {
    // Its own handle on the handler: the menu runs a message loop of its own,
    // and should the tray be dropped inside it, `state` is gone afterwards.
    let handler = state.handler.clone();
    // SAFETY: Win32 calls on this thread with our own live window and a menu
    // made and destroyed here; the item strings outlive `TrackPopupMenu`.
    let picked = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, PCWSTR(state.open.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(state.quit.as_ptr()));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // Without this the menu would not close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, at.x, at.y, None, hwnd, None);
        // Documented companion of the call above, so the next click works.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        picked.0 as usize
    };
    match picked {
        ID_OPEN => handler(Event::Open),
        ID_QUIT => handler(Event::Quit),
        _ => {}
    }
}

/// Receives the icon's callbacks. Anything before the state is attached, and
/// anything not ours, goes to the default procedure.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the pointer is either null or the box set in `Tray::new`, which
    // is only freed after this window is destroyed. It is read as a shared
    // reference, so a handler running inside the menu loop can come back here.
    let state = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State).as_ref() };
    let Some(state) = state else {
        // SAFETY: forwarding the same arguments we were given.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };

    if msg == CALLBACK_MESSAGE {
        // Classic format: the mouse message is in the low word of `lparam`.
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONUP => (state.handler)(Event::Open),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd, state),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == state.taskbar_created && msg != 0 {
        // Explorer restarted and the icon is gone: add it again.
        // SAFETY: `data` is the initialised struct used for the first add.
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &state.data) }.as_bool() {
            // SAFETY: reading the thread's last error right after the failed call.
            log::warn!("could not re-add the tray icon (error {:?})", unsafe { GetLastError() });
        }
        return LRESULT(0);
    }
    // SAFETY: forwarding the same arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
