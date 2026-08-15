//! Real app-icon extraction (Feature 4).
//!
//! `get_app_icon(exe_path)` extracts the Windows executable's icon, resizes it
//! to 48×48, encodes it as a PNG, and returns it as a base64 `data:image/png`
//! URL so the frontend can cache + send it once per exe over the WS
//! (`activity_icon`). Graceful degradation: returns `None` on any failure or on
//! non-Windows targets — the frontend then shows a `GeneratedAppIcon` fallback.
//!
//! The extraction path is the classic Win32 idiom:
//!   `SHGetFileInfoW(SHGFI_ICON | SHGFI_LARGEICON)` → HICON
//!   → `GetIconInfo` → HBITMAP color bitmap (DIB section)
//!   → `GetDIBits` → raw BGRA pixels
//!   → `image` crate decode of those pixels → resize → PNG → base64.
//!
//! Privacy: we only ever read the icon of an EXE that the user's OWN machine is
//! running — never the window title or any screen content. The icon crosses the
//! wire once per new exe so the partner renders the real program icon.
//!
//! NOTE on the `windows` crate version (0.58): GDI/Shell calls return a mix of
//! `Result<T>` and primitive `BOOL`/`i32`. We handle each defensively so a
//! per-icon failure degrades to `None` (never panics) and the feature stays
//! non-blocking from the frontend's point of view.

use serde::Serialize;

/// The foreground app, returned to the frontend. `exe` is the lowercased
/// basename (e.g. "code.exe") used as the activity key + icon cache key; `path`
/// is the full image-name (e.g. "C:\\...\\code.exe") used here to extract the
/// once-per-exe icon. Either may be `None` on failure / non-Windows.
#[derive(Serialize)]
pub struct ForegroundApp {
    pub exe: Option<String>,
    pub path: Option<String>,
}

/// Extract the icon for an exe PATH, returning a base64 `data:image/png` URL
/// (48×48). `None` on non-Windows or any extraction failure (→ generated fallback).
#[tauri::command]
pub fn get_app_icon(exe_path: String) -> Option<String> {
    #[cfg(windows)]
    {
        icon_windows(&exe_path)
    }
    #[cfg(not(windows))]
    {
        let _ = exe_path;
        None
    }
}

#[cfg(windows)]
fn icon_windows(exe_path: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

    // Wide path for the Win32 call.
    let mut wide: Vec<u16> = exe_path.encode_utf16().collect();
    wide.push(0);

    let mut info = SHFILEINFOW::default();
    // SHGFI_ICON | SHGFI_LARGEICON → a 32×32 (or jumbo-aware) HICON.
    let flags = SHGFI_ICON | SHGFI_LARGEICON; // SHGFI_FLAGS (BitOr)
    // SHGetFileInfoW returns the count of copied entries (usize); a zero result
    // means no icon was found. The windows 0.58 signature takes the path as a
    // Param<PCWSTR>, the file attrs as a newtype, the info struct as
    // Option<*mut>, and the flags as SHGFI_FLAGS (not a raw u32).
    let got = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info as *mut _),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if got == 0 || info.hIcon.is_invalid() {
        return None;
    }
    let hicon: HICON = info.hIcon;

    // Render the HICON into raw BGRA via GetIconInfo + GetDIBits, then encode.
    let result = icon_to_png_data_url(hicon);
    // Always release the icon handle.
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    result
}

/// Pull the icon's color bitmap pixels via GetDIBits and encode a 48×48 PNG as a
/// base64 data URL. Returns `None` on any GDI failure.
#[cfg(windows)]
fn icon_to_png_data_url(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<String> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    // GetIconInfo (windows 0.58) lives in WindowsAndMessaging (gated on the
    // Gdi feature) and fills an out-parameter `*mut ICONINFO` (not a return
    // value); it returns `Result<()>`, so `.ok()?` turns a failed read into None
    // (→ generated fallback). The caller owns the color + mask bitmaps it
    // fills in, so we must DeleteObject both below.
    let mut ii = ICONINFO::default();
    unsafe { GetIconInfo(hicon, &mut ii) }.ok()?;
    // Color bitmap (RGBA when present); may be absent for monochrome icons.
    let hbm_color = ii.hbmColor;
    // Mask bitmap is always present; used as a fallback when there's no color
    // bitmap (monochrome icon) and for alpha-aware rendering.
    let hbm_mask = ii.hbmMask;

    if hbm_color.is_invalid() {
        return None;
    }

    // Dimensions of the color bitmap.
    let (w, h) = bitmap_size(hbm_color)?;
    if w == 0 || h == 0 {
        return None;
    }

    // A CompatibleDC lets GetDIBits select + read the bitmap.
    let hdc = unsafe { CreateCompatibleDC(None) };
    if hdc.is_invalid() {
        return None;
    }

    // Prepare a 32-bit BGRA DIB header so GetDIBits returns alpha-aware pixels.
    let bih = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w as i32,
        biHeight: -(h as i32), // top-down DIB — rows in top-to-bottom order
        biPlanes: 1,
        biBitCount: 32,
        biCompression: 0, // BI_RGB
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut bmi = BITMAPINFO {
        bmiHeader: bih,
        bmiColors: [Default::default(); 1],
    };

    let mut pixels: Vec<u8> = vec![0u8; (w as usize) * (h as usize) * 4];
    let written = unsafe {
        GetDIBits(
            hdc,
            hbm_color,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        let _ = DeleteDC(hdc);
    }
    if written == 0 {
        return None;
    }

    // Free both bitmaps GetIconInfo gave us (we own them now).
    unsafe {
        let _ = DeleteObject(hbm_color);
        if !hbm_mask.is_invalid() {
            let _ = DeleteObject(hbm_mask);
        }
    }

    // The pixels are 32-bit BGRA. We transcode to RGBA for the `image` crate.
    let mut rgba = pixels;
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2); // BGRA → RGBA
    }

    rgb_to_png_data_url(rgba, w as u32, h as u32, 48, 48)
}

/// Read a GDI bitmap's pixel dimensions via GetObject + BITMAP. Cheap + avoids a
/// second GetDIBits probe round.
#[cfg(windows)]
fn bitmap_size(hbm: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(i32, i32)> {
    use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP};
    let mut bm = BITMAP::default();
    let n = unsafe { GetObjectW(hbm, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) };
    if n == 0 {
        return None;
    }
    Some((bm.bmWidth, bm.bmHeight))
}

/// Decode raw RGBA bytes → resize à target size → PNG → base64 data URL. Falls
/// back to the un-resized encode if the resize step fails (still a valid icon).
#[cfg(windows)]
fn rgb_to_png_data_url(
    rgba: Vec<u8>,
    w: u32,
    h: u32,
    out_w: u32,
    out_h: u32,
) -> Option<String> {
    use base64::Engine;
    use image::ImageBuffer;

    // Build an RGBA image from the raw bytes. (image 0.25 RgbaImage is generic
    // over container; Vec-backed here.) `from_raw` returns Option, and the `?`
    // propagates None into this fn's Option<String> return.
    let img: image::RgbaImage = ImageBuffer::from_raw(w, h, rgba)?;

    // Resize to 48×48 with the NEAREST filter — icons are small + crisp, so
    // nearest keeps edges clean and is very cheap.
    let small = image::imageops::resize(
        &img,
        out_w,
        out_h,
        image::imageops::FilterType::Nearest,
    );

    // Encode as PNG into a Vec<u8>.
    let mut out = std::io::Cursor::new(Vec::<u8>::new());
    image::DynamicImage::ImageRgba8(small)
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    let bytes = out.into_inner();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:image/png;base64,{}", b64))
}
