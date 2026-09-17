//! Dock the CostDog bar against the bottom edge of the ZCode desktop window.
//!
//! Why poll instead of hooking: the host app is closed-source Electron, so there
//! is no event surface; CoreGraphics' `CGWindowListCopyWindowInfo` reports every
//! on-screen window's bounds (owner name, layer, frame) without any permission
//! prompt, which makes a cheap 500ms poll the least-privileged way to follow it.
//! When the dock is off — or ZCode is not running — the bar simply stays where
//! the window-state plugin put it. Windows/Linux keep the floating bar until
//! their window-tracking equivalents are implemented.

use std::sync::atomic::{AtomicBool, Ordering};

static DOCK_ENABLED: AtomicBool = AtomicBool::new(false);

/// 当前停靠目标的客户端进程名（无目标/未启用时为空），供前端按宿主适配主题。
static CURRENT_HOST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_enabled(enabled: bool) {
    DOCK_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    DOCK_ENABLED.load(Ordering::Relaxed)
}

pub fn current_host() -> String {
    CURRENT_HOST.lock().map(|host| host.clone()).unwrap_or_default()
}

fn set_current_host(host: &str) {
    if let Ok(mut slot) = CURRENT_HOST.lock() {
        if *slot != host {
            *slot = host.to_string();
        }
    }
}

/// While docked, give the window a native macOS HUD material so the live strip
/// blends with whatever is underneath (the same trick NotchNook-style overlays
/// use to feel like part of the host app). The bar faces keep their opaque
/// skin backgrounds; only the translucent strip/popover regions pick it up.
/// Requires `"transparent": true` in tauri.conf.json.
#[cfg(target_os = "macos")]
pub fn apply_material(app: &tauri::AppHandle, on: bool) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if !on {
        window.set_effects(None).ok();
        return;
    }
    let effects = tauri::window::EffectsBuilder::new()
        .effects([tauri::window::Effect::HudWindow])
        .state(tauri::window::EffectState::FollowsWindowActiveState)
        .build();
    window.set_effects(Some(effects)).ok();
}

#[cfg(not(target_os = "macos"))]
pub fn apply_material(_app: &tauri::AppHandle, _on: bool) {}

#[cfg(target_os = "macos")]
pub fn spawn(app: tauri::AppHandle) {
    std::thread::spawn(move || dock_loop(app));
}

#[cfg(not(target_os = "macos"))]
pub fn spawn(_app: tauri::AppHandle) {}

/// 已知 AI 编程客户端的进程名清单（CGWindowList owner 精确匹配）。
/// 未来由 app_prefs 的 clients 配置驱动，当前内置最小集。
const KNOWN_CLIENTS: &[&str] = &["ZCode", "DeepSeek Harness", "Claude", "Codex", "Cursor", "Windsurf"];

/// 前台跟随防抖：候选目标需连续出现在该次数的轮询中才切换（2×500ms ≈ 1s，
/// 与设计规范 focus_hysteresis_ms=500 同量级，覆盖 Cmd+Tab 掠过）。
const DEBOUNCE_TICKS: u32 = 2;

#[cfg(target_os = "macos")]
fn dock_loop(app: tauri::AppHandle) {
    use tauri::Manager;

    // Skip repositioning when the frame has not changed: every set_position
    // costs a native window move and would fight the window-state plugin.
    let mut last_applied: Option<(f64, f64)> = None;
    let mut current: Option<String> = None;
    let mut candidate: Option<String> = None;
    let mut candidate_ticks: u32 = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if !enabled() {
            last_applied = None;
            continue;
        }
        // 前台检测：屏幕上从前到后第一个已知客户端窗口即为候选目标；
        // 防抖通过后才切换，未通过期间继续跟随原目标（或原地等待）。
        let scanned = find_dock_target();
        let owner = match &scanned {
            Some((name, ..)) => {
                if candidate.as_deref() == Some(name.as_str()) {
                    candidate_ticks += 1;
                } else {
                    candidate = Some(name.clone());
                    candidate_ticks = 1;
                }
                if candidate_ticks >= DEBOUNCE_TICKS {
                    Some(name.clone())
                } else {
                    current.clone()
                }
            }
            None => current.clone(), // 无已知客户端在前台：保持在最近位置
        };
        let Some(owner) = owner else { continue };
        let Some((_, zx, zy, zw, zh)) = scanned.filter(|(name, ..)| *name == owner) else {
            continue;
        };
        let Some(window) = app.get_webview_window("main") else {
            continue;
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let dog_width = window.outer_size().map_or(410.0, |s| s.width as f64 / scale);
        let dog_height = window.outer_size().map_or(36.0, |s| s.height as f64 / scale);

        // Attach just below ZCode's bottom edge with a 1px overlap so the bar's
        // 嵌入模式：放进宿主底部输入栏「权限/模型选择器之间的空白区域」。
        // 线性标定模型（w=1820/1200 两点实测，Gemini 截图测量）：
        //   空隙左缘 = 0.3774w + 23.1，右缘 = 1.2742w − 752.1，
        //   中心 = 0.8258w − 364.5；Electron flexbox 布局下随宽度线性伸缩。
        // 容纳性校验：空隙放不下 410px 胶囊（+16px 余量）时冻结在最近有效
        // 位置，避免压住两侧芯片。
        let gap_left = 0.3774 * zw + 23.1;
        let gap_right = 1.2742 * zw - 752.1;
        if gap_right - gap_left < dog_width + 16.0 {
            continue;
        }
        // AX 精确锚定（2026-09-17）：左右芯片簇锚定两侧、宽度固定，
        // 空隙中点 = 0.5w − 19.5；文字中心与芯片文字中心（距底 48px）对齐，
        // 即胶囊顶距底 36px。
        let mut x = zx + 0.5 * zw - 19.5 - dog_width / 2.0;
        let min_x = gap_left + 8.0;
        let max_x = gap_right - dog_width - 8.0;
        if x < min_x {
            x = min_x;
        }
        if x > max_x {
            x = max_x;
        }
        // 锚定主行（首行 24px）：文字中心与芯片文字中心一致（距底 48px），
        // 次级行/展开面板向下自然延展。
        let mut y = zy + zh - 24.0 - 36.0;
        if y < zy {
            y = zy;
        }
        if last_applied == Some((x, y)) {
            continue;
        }
        // 200ms 平滑滑入（设计规范 dock_animation_ms）：分 5 帧插值，
        // 目标切换或首次停靠时生效；微小修正直接落位不抖动。
        let from = last_applied.unwrap_or((x, y));
        const STEPS: u32 = 5;
        let step_x = (x - from.0) / STEPS as f64;
        let step_y = (y - from.1) / STEPS as f64;
        let mut moved_ok = true;
        for step in 1..=STEPS {
            let last = step == STEPS;
            let ix = if last { x } else { from.0 + step_x * step as f64 };
            let iy = if last { y } else { from.1 + step_y * step as f64 };
            if window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x: ix, y: iy }))
                .is_err()
            {
                moved_ok = false;
                break;
            }
            if !last {
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
        if moved_ok {
            last_applied = Some((x, y));
            current = Some(owner.clone());
            set_current_host(&owner);
        }
    }
}

/// 前台最靠前的已知客户端窗口：(owner, x, y, width, height)，CG 全局坐标
/// （左上原点）。窗口列表本身按前到后排序，因此第一个命中的已知客户端
/// 就是前台目标。
#[cfg(target_os = "macos")]
fn find_dock_target() -> Option<(String, f64, f64, f64, f64)> {
    unsafe {
        let list = ffi::CGWindowListCopyWindowInfo(ffi::ON_SCREEN_ONLY, 0);
        if list.is_null() {
            return None;
        }
        let count = ffi::CFArrayGetCount(list);
        // 窗口列表按前到后排序：每个已知客户端记录它最靠前的最大窗口；
        // 全部扫完后取排位最靠前的客户端作为停靠目标。
        let mut best: Option<(usize, String, f64, f64, f64, f64)> = None;
        for (order, index) in (0..count).enumerate() {
            let dict = ffi::CFArrayGetValueAtIndex(list, index);
            if dict.is_null() {
                continue;
            }
            let owner_ref = ffi::CFDictionaryGetValue(dict, ffi::kCGWindowOwnerName);
            if owner_ref.is_null() {
                continue;
            }
            let Some(owner) = cf_string(owner_ref) else { continue };
            if !KNOWN_CLIENTS.contains(&owner.as_str()) {
                continue;
            }
            // Layer 0 = normal window; helpers, overlays and the always-on-top
            // CostDog bar itself live on other layers.
            if number_value(ffi::CFDictionaryGetValue(dict, ffi::kCGWindowLayer)) != Some(0.0) {
                continue;
            }
            let bounds = ffi::CFDictionaryGetValue(dict, ffi::kCGWindowBounds);
            if bounds.is_null() {
                continue;
            }
            let (Some(x), Some(y), Some(width), Some(height)) = (
                number_value(ffi::CFDictionaryGetValue(bounds, bounds_x())),
                number_value(ffi::CFDictionaryGetValue(bounds, bounds_y())),
                number_value(ffi::CFDictionaryGetValue(bounds, bounds_width())),
                number_value(ffi::CFDictionaryGetValue(bounds, bounds_height())),
            ) else {
                continue;
            };
            if width < 200.0 || height < 200.0 {
                continue;
            }
            match &best {
                Some((_, best_owner, bx, by, bw, bh)) if *best_owner == owner => {
                    if width * height > bw * bh {
                        best = Some((order, owner, x, y, width, height));
                    }
                }
                Some(_) => {} // 更靠前的已知客户端已锁定，忽略更靠后的
                None => best = Some((order, owner, x, y, width, height)),
            }
        }
        ffi::CFRelease(list);
        best.map(|(_, owner, x, y, width, height)| (owner, x, y, width, height))
    }
}

#[cfg(target_os = "macos")]
fn cf_string(reference: ffi::CFTypeRef) -> Option<String> {
    let mut buffer = [0u8; 256];
    let ok = unsafe {
        ffi::CFStringGetCString(
            reference,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            ffi::K_CF_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    String::from_utf8(buffer[..end].to_vec()).ok()
}

#[cfg(target_os = "macos")]
fn number_value(reference: ffi::CFTypeRef) -> Option<f64> {
    if reference.is_null() {
        return None;
    }
    let mut value = 0f64;
    let ok = unsafe { ffi::CFNumberGetValue(reference, ffi::K_CF_FLOAT64, &mut value) };
    (ok != 0).then_some(value)
}

/// Raw CoreGraphics/CoreFoundation FFI. Hand-rolled instead of pulling in
/// binding crates: the surface used here is a dozen stable C functions.
#[cfg(target_os = "macos")]
mod ffi {
    use std::os::raw::{c_double, c_void};

    pub type CFTypeRef = *const c_void;

    pub const ON_SCREEN_ONLY: u32 = 1; // kCGWindowListOptionOnScreenOnly
    pub const K_CF_UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
    pub const K_CF_FLOAT64: i32 = 6; // kCFNumberFloat64Type

    extern "C" {
        pub fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: u32) -> CFTypeRef;
        pub fn CFArrayGetCount(array: CFTypeRef) -> isize;
        pub fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
        pub fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        pub fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut u8,
            bufferSize: isize,
            encoding: u32,
        ) -> u8;
        pub fn CFNumberGetValue(number: CFTypeRef, theType: i32, valuePtr: *mut c_double) -> u8;
        pub fn CFRelease(reference: CFTypeRef);

        // Exported CFString constants; read as raw pointers, never dereferenced in Rust.
        pub static kCGWindowOwnerName: CFTypeRef;
        pub static kCGWindowBounds: CFTypeRef;
        pub static kCGWindowLayer: CFTypeRef;
    }

    // Window-bounds dictionary keys are plain CFStrings, not exported symbols:
    // create them once and leak — they live for the process lifetime anyway.
    // (Raw pointers are not Send/Sync, so the handle is stored as usize.)
    macro_rules! leaked_cfstring {
        ($fn_name:ident, $value:expr) => {
            pub fn $fn_name() -> CFTypeRef {
                static HANDLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
                *HANDLE.get_or_init(|| {
                    let mut bytes = $value.as_bytes().to_vec();
                    bytes.push(0);
                    unsafe {
                        CFStringCreateWithCString(std::ptr::null(), bytes.as_ptr(), K_CF_UTF8)
                    }
                } as usize) as CFTypeRef
            }
        };
    }

    extern "C" {
        pub fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            bytes: *const u8,
            encoding: u32,
        ) -> CFTypeRef;
    }

    leaked_cfstring!(bounds_x, "X");
    leaked_cfstring!(bounds_y, "Y");
    leaked_cfstring!(bounds_width, "Width");
    leaked_cfstring!(bounds_height, "Height");
}

#[cfg(target_os = "macos")]
use ffi::{bounds_height, bounds_width, bounds_x, bounds_y};
