#[cfg(windows)]
pub fn apply() {
    use core::ffi::c_void;

    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const IMAGE_ICON: u32 = 1;
    const WM_SETICON: u32 = 0x0080;
    const PETRI_ICON_RESOURCE_ID: usize = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut c_void;
        fn GetModuleHandleW(module_name: *const u16) -> *mut c_void;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn LoadImageW(
            instance: *mut c_void,
            name: *const u16,
            image_type: u32,
            width: i32,
            height: i32,
            load_flags: u32,
        ) -> *mut c_void;
        fn SendMessageW(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
    }

    // The icon handles intentionally live for the process lifetime because the
    // console window keeps using them after WM_SETICON returns.
    unsafe {
        let window = GetConsoleWindow();
        let module = GetModuleHandleW(std::ptr::null());
        if window.is_null() || module.is_null() {
            return;
        }

        let resource = PETRI_ICON_RESOURCE_ID as *const u16;
        let large_icon = LoadImageW(module, resource, IMAGE_ICON, 32, 32, 0);
        let small_icon = LoadImageW(module, resource, IMAGE_ICON, 16, 16, 0);

        if !large_icon.is_null() {
            SendMessageW(window, WM_SETICON, ICON_BIG, large_icon as isize);
        }
        if !small_icon.is_null() {
            SendMessageW(window, WM_SETICON, ICON_SMALL, small_icon as isize);
        }
    }
}

#[cfg(not(windows))]
pub fn apply() {}
