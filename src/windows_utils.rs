use std::ffi::CString;

use windows::{
    core::{s, PCSTR},
    Win32::{
        Foundation::MAX_PATH,
        System::{
            Console::{AttachConsole, FreeConsole, ATTACH_PARENT_PROCESS},
            LibraryLoader::GetModuleFileNameA,
            Registry::{
                RegCloseKey, RegCreateKeyExA, RegDeleteTreeA, RegSetValueExA, HKEY,
                HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
            },
        },
        UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
    },
};

fn get_binary_path() -> String {
    let mut buffer = [0u8; MAX_PATH as usize];
    unsafe {
        GetModuleFileNameA(None, &mut buffer);
        PCSTR::from_raw(buffer.as_ptr()).to_string().unwrap()
    }
}

pub fn unregister_rightclick() -> bool {
    let str_registry_path_1 = s!("Software\\Classes\\Directory\\Background\\shell\\Neovide");
    let str_registry_path_2 = s!("Software\\Classes\\*\\shell\\Neovide");
    unsafe {
        let s1 = RegDeleteTreeA(HKEY_CURRENT_USER, str_registry_path_1);
        let s2 = RegDeleteTreeA(HKEY_CURRENT_USER, str_registry_path_2);
        s1.is_ok() && s2.is_ok()
    }
}

pub fn register_rightclick(
    registry_path: PCSTR,
    registry_command_path: PCSTR,
    command: CString,
) -> bool {
    let neovide_path = get_binary_path();
    let mut registry_key = HKEY::default();
    let str_icon = s!("Icon");
    let str_description = CString::new("Open with Neovide").unwrap();
    let str_neovide_path = CString::new(neovide_path).unwrap();
    unsafe {
        if RegCreateKeyExA(
            HKEY_CURRENT_USER,
            registry_path,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut registry_key,
            None,
        )
        .is_err()
        {
            RegCloseKey(registry_key);
            return false;
        }
        let registry_values = [
            (PCSTR::null(), REG_SZ, str_description.as_bytes_with_nul()),
            (str_icon, REG_SZ, str_neovide_path.as_bytes_with_nul()),
        ];
        for &(key, keytype, value) in &registry_values {
            RegSetValueExA(registry_key, key, 0, keytype, Some(value));
        }
        RegCloseKey(registry_key);

        if RegCreateKeyExA(
            HKEY_CURRENT_USER,
            registry_command_path,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut registry_key,
            None,
        )
        .is_err()
        {
            return false;
        }
        let registry_values = [(PCSTR::null(), REG_SZ, command.as_bytes_with_nul())];
        for &(key, keytype, value) in &registry_values {
            RegSetValueExA(registry_key, key, 0, keytype, Some(value));
        }
        RegCloseKey(registry_key);
    }
    true
}

pub fn register_rightclick_directory() -> bool {
    let neovide_path = get_binary_path();
    let registry_path = s!("Software\\Classes\\Directory\\Background\\shell\\Neovide");
    let registry_command_path =
        s!("Software\\Classes\\Directory\\Background\\shell\\Neovide\\command");
    let command = CString::new(format!("{} \"%V\"", neovide_path)).unwrap();
    register_rightclick(registry_path, registry_command_path, command)
}

pub fn register_rightclick_file() -> bool {
    let neovide_path = get_binary_path();
    let registry_path = s!("Software\\Classes\\*\\shell\\Neovide");
    let registry_command_path = s!("Software\\Classes\\*\\shell\\Neovide\\command");
    let command = CString::new(format!("{} \"%1\"", neovide_path).as_bytes()).unwrap();
    register_rightclick(registry_path, registry_command_path, command)
}

pub fn windows_fix_dpi() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn windows_attach_to_console() {
    // Attach to parent console tip found here: https://github.com/rust-lang/rust/issues/67159#issuecomment-987882771
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn windows_detach_from_console() {
    unsafe {
        FreeConsole();
    }
}
