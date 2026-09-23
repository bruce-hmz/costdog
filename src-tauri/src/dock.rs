//! Dock the CostDog bar against the bottom edge of the ZCode desktop window.
//!
//! Why poll instead of hooking: the host app is closed-source Electron, so there
//! is no event surface; CoreGraphics' `CGWindowListCopyWindowInfo` reports every
//! on-screen window's bounds (owner name, layer, frame) without any permission
//! prompt, which makes a cheap 500ms poll the least-privileged way to follow it.
//! When the dock is off — or ZCode is not running — the bar simply stays where
//! the window-state plugin put it. Windows/Linux keep the floating bar until
//! their window-tracking equivalents are implemented.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static DOCK_ENABLED: AtomicBool = AtomicBool::new(false);

/// 把窗口变为 non-activating panel：点击内容不激活应用、事件直达 webview，
/// 消除"第一次点击只用于激活窗口"的 macOS 默认行为（悬浮 HUD 的通行做法）。
#[cfg(target_os = "macos")]
pub fn make_non_activating(window: &tauri::WebviewWindow) {
    let Ok(ns_window) = window.ns_window() else { return };
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        let obj = ns_window as *mut AnyObject;
        let mask: usize = msg_send![obj, styleMask];
        // NSNonactivatingPanelMask = 1 << 7
        let _: () = msg_send![obj, setStyleMask: mask | (1 << 7)];
    }
}

static CURRENT_HOST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// 每个胶囊窗口（label→宿主）的宿主名，供 get_dock_host 按窗口查询。
static HOST_BY_LABEL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub fn host_for_label(label: &str) -> String {
    HOST_BY_LABEL
        .lock()
        .ok()
        .and_then(|map| map.get(label).cloned())
        .unwrap_or_default()
}

fn register_host_label(label: &str, host: &str) {
    if let Ok(mut map) = HOST_BY_LABEL.lock() {
        map.insert(label.to_string(), host.to_string());
    }
}

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
const KNOWN_CLIENTS: &[&str] = &["ZCode", "DeepSeek Harness", "Claude", "Codex", "ChatGPT", "Cursor", "Windsurf"];

/// 前台跟随防抖：候选目标需连续出现在该次数的轮询中才切换（2×500ms ≈ 1s，
/// 与设计规范 focus_hysteresis_ms=500 同量级，覆盖 Cmd+Tab 掠过）。
const DEBOUNCE_TICKS: u32 = 2;

#[cfg(target_os = "macos")]
fn dock_loop(app: tauri::AppHandle) {
    use tauri::Manager;
    eprintln!("[CostDog] dock loop thread started, enabled={}", enabled());

    // Skip repositioning when the frame has not changed: every set_position
    // costs a native window move and would fight the window-state plugin.
    let mut last_applied: Option<(f64, f64)> = None;
    let mut current: Option<String> = None;
    let mut candidate: Option<String> = None;
    let mut candidate_ticks: u32 = 0;
    let mut hidden = false;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if !enabled() {
            last_applied = None;
            continue;
        }
        // 前台检测：屏幕上从前到后第一个已知客户端窗口即为候选目标；
        // 防抖通过后才切换，未通过期间继续跟随原目标（或原地等待）。
        let Some(window) = app.get_webview_window("main") else {
            continue;
        };
        // ChatGPT 专属胶囊每 tick 无条件管理（显示/隐藏/定位）——必须先于
        // 本循环所有 continue 路径执行，否则它会脱管残留桌面（用户实测）。
        manage_chatgpt_capsule(&app);
        // 前台应用守卫：宿主不在前台（用户切到浏览器/Finder 等）时隐藏，
        // 置顶胶囊绝不能盖在其他应用上；CostDog 自身获得焦点时豁免。
        let front = frontmost_owner();
        let host_front = front.as_deref().is_some_and(|name| {
            KNOWN_CLIENTS.contains(&name) || name.eq_ignore_ascii_case("costdog")
        });
        let scanned = if host_front { find_dock_target() } else { None };
        // 目标窗口不在屏幕上（最小化/关闭/切到其他 Space/宿主非前台）：隐藏胶囊，
        // 避免残留在桌面遮挡其他应用；目标回到屏幕后自动恢复。
        if scanned.is_none() && current.is_some() {
            if !hidden {
                window.hide().ok();
                hidden = true;
            }
            last_applied = None;
            continue;
        }
        if hidden && scanned.is_some() {
            window.show().ok();
            hidden = false;
        }
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
        // 诊断日志：仅状态变化时打印（owner/前台/隐藏）。
        static LAST_LOG: std::sync::Mutex<(String, bool)> = std::sync::Mutex::new((String::new(), false));
        {
            let hidden_now = hidden;
            let mut last = LAST_LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if last.0 != owner || last.1 != hidden_now {
                eprintln!("[CostDog] owner={} hidden={} front={:?}", owner, hidden_now, front);
                *last = (owner.clone(), hidden_now);
            }
        }
        // 多胶囊：主窗口固定服务 ZCode。其他宿主（ChatGPT 有专属胶囊；
        // DSH 等无布局适配的客户端直接隐藏）不再让 main 飞来飞去。
        if owner != "ZCode" {
            if let Some(w) = app.get_webview_window("main") {
                if !hidden {
                    w.hide().ok();
                    hidden = true;
                }
            }
            last_applied = None;
            continue;
        }
        let Some((_, zx, zy, zw, zh)) = scanned.filter(|(name, ..)| *name == owner) else {
            continue;
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let dog_width = window.outer_size().map_or(410.0, |s| s.width as f64 / scale);

        // Attach just below ZCode's bottom edge with a 1px overlap so the bar's
        // 嵌入模式几何，按宿主客户端分支：
        //   ZCode：AX 标定芯片行布局（左簇右缘 493 / 右簇随模型名动态 / 锚定左芯片+46px）
        //   ChatGPT(Codex Desktop)：无同款芯片行，v1 用窗口内右下角 16px
        let host_lc = owner.to_ascii_lowercase();
        let is_chatgpt = host_lc == "chatgpt";
        let mut target_w = 410.0f64;
        let mut anchor_x;
        let mut anchor_y;
        if !is_chatgpt {
            let gap_left = 493.0;
            let model_len = crate::current_model_name().map(|name| name.chars().count()).unwrap_or(13);
            let right_zone = (470.0 + model_len as f64 * 6.5).clamp(500.0, 780.0);
            let gap_right = zw - right_zone;
            let gap_w = gap_right - gap_left;
            if gap_w < 316.0 {
                if !hidden {
                    window.hide().ok();
                    hidden = true;
                }
                last_applied = None;
                continue;
            }
            if hidden {
                window.show().ok();
                hidden = false;
            }
            let tw = (gap_w - 40.0).clamp(300.0, 410.0);
            target_w = tw;
            // 锚定左芯片右侧 46px（用户红框标定）；防与右侧芯片重叠保 20px。
            let mut ax = zx + gap_left + 46.0;
            let max_left = zx + gap_right - 20.0 - tw;
            if ax > max_left {
                ax = max_left;
            }
            anchor_x = ax;
            anchor_y = zy + zh - 24.0 - 36.0;
        } else {
            // ChatGPT：右下角内侧 16px；窗口过窄（<450）时隐藏。
            if zw < 450.0 {
                if !hidden {
                    window.hide().ok();
                    hidden = true;
                }
                last_applied = None;
                continue;
            }
            if hidden {
                window.show().ok();
                hidden = false;
            }
            target_w = 410.0;
            anchor_x = zx + zw - target_w - 16.0;
            anchor_y = zy + zh - 24.0 - 16.0;
        }
        if (dog_width - target_w).abs() > 1.0 {
            let cur_h = window.outer_size().map_or(24.0, |sz| sz.height as f64 / scale);
            window
                .set_size(tauri::Size::Logical(tauri::LogicalSize { width: target_w, height: cur_h }))
                .ok();
        }
        let x = anchor_x;
        // 锚定主行：ZCode 与芯片文字中心对齐（距底 60px 含边框）；ChatGPT 右下角同高。
        let mut y = anchor_y;
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

/// ChatGPT 的独立胶囊窗口：存在性、定位（右下角 16px）、显示/隐藏。
/// 每 tick 调用；前台非 ChatGPT 或其窗口不在屏 → 隐藏。
fn manage_chatgpt_capsule(app: &tauri::AppHandle) {
    use tauri::Manager;
    let label = "cap-chatgpt";
    let front = frontmost_owner();
    let should_show = front.as_deref() == Some("ChatGPT");
    let window = match app.get_webview_window(label) {
        Some(w) => w,
        None => {
            if !should_show {
                return;
            }
            // 首次创建：同款胶囊外观参数。窗口创建必须派发到主线程——
            // 从 dock 线程直接 build 会死锁（症状：主窗口 hide 后永不恢复）。
            let app2 = app.clone();
            let label_owned = label.to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            let dispatch_ok = app.run_on_main_thread(move || {
                let built = tauri::WebviewWindowBuilder::new(
                    &app2,
                    &label_owned,
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .title("CostDog")
                .inner_size(410.0, 36.0)
                .decorations(false)
                .always_on_top(true)
                .resizable(false)
                .skip_taskbar(true)
                .visible(false)
                .build();
                let ok = match built {
                    Ok(w) => {
                        w.set_shadow(false).ok();
                        #[cfg(target_os = "macos")]
                        make_non_activating(&w);
                        register_host_label(label, "chatgpt");
                        true
                    }
                    Err(error) => {
                        eprintln!("[CostDog] capsule window build failed: {error}");
                        false
                    }
                };
                let _ = tx.send(ok);
            });
            if dispatch_ok.is_err() || rx.recv_timeout(std::time::Duration::from_secs(5)) != Ok(true) {
                return;
            }
            app.get_webview_window(label).unwrap()
        }
    };
    if !should_show {
        if window.is_visible().unwrap_or(false) {
            window.hide().ok();
        }
        return;
    }
    // 定位：ChatGPT 自己的窗口右下角 16px（此前误用"最靠前已知客户端"的
    // 边界——前台是 ZCode 时会拿到 ZCode 的框，位置错乱）。
    if let Some((_, zx, zy, zw, zh)) = find_known_window(Some("ChatGPT")) {
        let x = zx + zw - 410.0 - 16.0;
        let y = zy + zh - 60.0;
        window
            .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .ok();
        if !window.is_visible().unwrap_or(false) {
            window.show().ok();
        }
    } else {
        // ChatGPT 窗口不在屏幕上（最小化/关窗）：藏起 cap 胶囊。
        if window.is_visible().unwrap_or(false) {
            window.hide().ok();
        }
    }
}

/// 屏幕最前方 layer-0 窗口的 owner（≈前台应用）；无窗口时 None。
/// CostDog 自身豁免：用户点开胶囊/面板时焦点转移到 CostDog，不算"离开宿主"。
#[cfg(target_os = "macos")]
fn frontmost_owner() -> Option<String> {
    unsafe {
        let list = ffi::CGWindowListCopyWindowInfo(ffi::ON_SCREEN_ONLY, 0);
        if list.is_null() {
            return None;
        }
        let count = ffi::CFArrayGetCount(list);
        let mut front: Option<String> = None;
        for index in 0..count {
            let dict = ffi::CFArrayGetValueAtIndex(list, index);
            if dict.is_null() {
                continue;
            }
            if number_value(ffi::CFDictionaryGetValue(dict, ffi::kCGWindowLayer)) != Some(0.0) {
                continue;
            }
            let owner_ref = ffi::CFDictionaryGetValue(dict, ffi::kCGWindowOwnerName);
            if owner_ref.is_null() {
                continue;
            }
            if let Some(owner) = cf_string(owner_ref) {
                front = Some(owner);
            }
            break;
        }
        ffi::CFRelease(list);
        front
    }
}

/// 前台最靠前的已知客户端窗口：(owner, x, y, width, height)，CG 全局坐标
/// （左上原点）。窗口列表本身按前到后排序，因此第一个命中的已知客户端
/// 就是前台目标。
#[cfg(target_os = "macos")]
fn find_dock_target() -> Option<(String, f64, f64, f64, f64)> {
    find_known_window(None)
}

/// 指定 owner（None=任意已知客户端）时，屏幕上最靠前的该客户端主窗口。
fn find_known_window(filter: Option<&str>) -> Option<(String, f64, f64, f64, f64)> {
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
            if let Some(want) = filter {
                if owner != want {
                    continue;
                }
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
