use std::mem::zeroed;
use std::ptr;

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP,
};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

use crate::IconPixels;

/// Call once on the thread that will call `app_icon`; the shell needs COM initialized there.
pub fn init_icon_thread() {
    unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
}

/// The large icon the shell shows for `path` (for a shortcut, its target's icon), as RGBA.
pub fn app_icon(path: &str) -> Option<IconPixels> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut info: SHFILEINFOW = unsafe { zeroed() };
    let found = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if found == 0 || info.hIcon.is_null() {
        return None;
    }
    let pixels = icon_pixels(info.hIcon);
    unsafe { DestroyIcon(info.hIcon) };
    pixels
}

/// Converts an HICON to RGBA. Icons without an alpha channel take transparency from their mask.
fn icon_pixels(icon: HICON) -> Option<IconPixels> {
    let mut parts: ICONINFO = unsafe { zeroed() };
    if unsafe { GetIconInfo(icon, &mut parts) } == 0 {
        return None;
    }
    let color = bitmap_bgra(parts.hbmColor);
    let mask = bitmap_bgra(parts.hbmMask);
    unsafe {
        DeleteObject(parts.hbmColor);
        DeleteObject(parts.hbmMask);
    }
    let (width, height, mut pixels) = color?;
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        let (_, _, mask) = mask?;
        for (pixel, bit) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            pixel[3] = if bit[0] == 0 { 255 } else { 0 };
        }
    }
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(IconPixels {
        width,
        height,
        rgba: pixels,
    })
}

fn bitmap_bgra(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if bitmap.is_null() {
        return None;
    }
    let mut header: BITMAP = unsafe { zeroed() };
    let size = size_of::<BITMAP>() as i32;
    if unsafe { GetObjectW(bitmap, size, (&mut header as *mut BITMAP).cast()) } == 0 {
        return None;
    }
    let (width, height) = (header.bmWidth, header.bmHeight);
    if width <= 0 || height <= 0 {
        return None;
    }
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { zeroed() }
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    unsafe { DeleteDC(dc) };
    (lines == height).then_some((width as u32, height as u32, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_has_a_visible_square_icon() {
        init_icon_thread();
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let icon = app_icon(&format!(r"{windows}\explorer.exe")).unwrap();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 16);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn missing_files_have_no_icon() {
        init_icon_thread();
        assert!(app_icon(r"C:\definitely\not\here\nothing.exe").is_none());
    }
}
