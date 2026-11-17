//! Put one image on the system clipboard (the image viewer's "Sao chép ảnh").
//!
//! Input is always PNG bytes (the HD image cache already stores PNG). Every call blocks on
//! OS clipboard APIs, so callers run it on a helper thread, never inside the egui frame.

/// Copy PNG bytes to the clipboard as an image.
pub(super) fn copy_png(png: &[u8]) -> Result<(), String> {
    if png.is_empty() {
        return Err("ảnh rỗng".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        super::macos_clipboard::write_png(png)
    }
    #[cfg(target_os = "windows")]
    {
        windows::copy_png(png)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        linux::copy_png(png)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    const CF_DIB: u32 = 8;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(h_wnd: *mut std::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn RegisterClipboardFormatA(name: *const u8) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut std::ffi::c_void;
        fn GlobalLock(mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn GlobalUnlock(mem: *mut std::ffi::c_void) -> i32;
        fn GlobalFree(mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    /// Allocate a movable global block holding `bytes`; `None` on allocation failure.
    fn alloc_global(bytes: &[u8]) -> Option<*mut std::ffi::c_void> {
        // SAFETY: the block is sized for `bytes`, locked only while copying, and freed here on
        // failure; on success the clipboard takes ownership.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if h.is_null() {
                return None;
            }
            let dst = GlobalLock(h);
            if dst.is_null() {
                GlobalFree(h);
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst as *mut u8, bytes.len());
            GlobalUnlock(h);
            Some(h)
        }
    }

    /// 32-bit bottom-up `BITMAPINFOHEADER` + BGRA pixels (`CF_DIB`).
    fn dib_from_rgba(img: &image::RgbaImage) -> Vec<u8> {
        let (w, h) = (img.width(), img.height());
        let mut out = Vec::with_capacity(40 + (w * h * 4) as usize);
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&(w * h * 4).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        for y in (0..h).rev() {
            for x in 0..w {
                let p = img.get_pixel(x, y).0;
                out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        out
    }

    pub(super) fn copy_png(png: &[u8]) -> Result<(), String> {
        let rgba = image::load_from_memory(png)
            .map_err(|e| format!("không đọc được ảnh: {e}"))?
            .to_rgba8();
        let dib = dib_from_rgba(&rgba);
        // SAFETY: standard clipboard sequence; handles passed to SetClipboardData are owned by
        // the system afterwards and freed by us only when the call fails.
        unsafe {
            let mut opened = false;
            for _ in 0..10 {
                if OpenClipboard(std::ptr::null_mut()) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            if !opened {
                return Err("không mở được clipboard".to_string());
            }
            EmptyClipboard();
            let mut ok = false;
            if let Some(h) = alloc_global(&dib) {
                if SetClipboardData(CF_DIB, h).is_null() {
                    GlobalFree(h);
                } else {
                    ok = true;
                }
            }
            let cf_png = RegisterClipboardFormatA(b"PNG\0".as_ptr());
            if cf_png != 0 {
                if let Some(h) = alloc_global(png) {
                    if SetClipboardData(cf_png, h).is_null() {
                        GlobalFree(h);
                    } else {
                        ok = true;
                    }
                }
            }
            CloseClipboard();
            if ok {
                Ok(())
            } else {
                Err("clipboard từ chối dữ liệu ảnh".to_string())
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod linux {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn pipe_to(mut cmd: Command, png: &[u8]) -> Result<(), String> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(png).map_err(|e| e.to_string())?;
        }
        // `xclip` / `wl-copy` stay alive to serve the selection, so do not wait for them.
        Ok(())
    }

    pub(super) fn copy_png(png: &[u8]) -> Result<(), String> {
        let mut wl = Command::new("wl-copy");
        wl.args(["--type", "image/png"]);
        if pipe_to(wl, png).is_ok() {
            return Ok(());
        }
        let mut xc = Command::new("xclip");
        xc.args(["-selection", "clipboard", "-t", "image/png", "-i"]);
        pipe_to(xc, png).map_err(|_| "cần cài wl-clipboard hoặc xclip để sao chép ảnh".to_string())
    }
}
