//! 轻量模式的原生窗口：Win32 + Direct2D 自绘，不经过任何 WebView。
//!
//! 线程模型：窗口跑在自己的线程上，有自己的 `GetMessage` 循环。不挂到 Tauri 的主线程，
//! 因为那条 tao 的消息泵有自己的节奏，原生窗口卡在它后面就没意义了。D2D/DWrite/WIC
//! 的工厂、`Engine`、HWND 全部在这条线程上创建，也只在这条线程上用。
//!
//! 这个模块不认识 Tauri。想回到完整模式只有一条出口：`on_return_to_full` 回调。
//! 关闭按钮也走同一条出口——是退进程还是把 WebView 拉回来，由调用方决定。
//!
//! 渲染目标的 DPI 固定成 96，于是 DIP 就等于设备像素，缩放全部由我们自己按
//! `GetDpiForWindow` 算。这样布局里只有一套坐标系，不用在两套单位之间来回换算。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;

use windows::core::{PCWSTR, BOOL};
use windows::Win32::Foundation::{
    GENERIC_READ, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM, D2DERR_RECREATE_TARGET,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, ScreenToClient, PAINTSTRUCT,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnLoad,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, CoCreateInstance, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RIGHT,
    VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SystemParametersInfoW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HTCAPTION,
    IDC_ARROW, MSG, SPI_GETWORKAREA, SWP_NOMOVE, SWP_NOZORDER, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW,
    WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_POPUP,
};

use super::shared::layout::Rect;
use super::shared::lyrics::LyricTrack;
use super::shared::state::{Action, Playback};
use super::shared::{AudioBackend, MediaControls, NowPlaying, RodioBackend, UriResolver};
use super::shared::{fmt_time, Drag, Fallback, Icon, Layout, PaintKey, LOGICAL_HEIGHT, LOGICAL_WIDTH};
use super::snapshot::{self, MiniSnapshot};
use super::LiteAction;

const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 100;

/// 直接造一个 Direct2D 矩形。绘制代码里到处是临时矩形，几何计算另在
/// `shared::layout`，两者不冲突：那边算坐标，这边只是画之前包一层。
fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// 把平台中立的矩形翻译成 Direct2D 的。
fn d2d(area: Rect) -> D2D_RECT_F {
    rect(area.left, area.top, area.right, area.bottom)
}

/// 用圆角矩形画圆。`D2D1_ELLIPSE` 的圆心字段是 `windows_numerics::Vector2`，
/// 那个类型不在 `windows` crate 的公开路径上，为了几个圆点多拉一个依赖不值当。
fn circle(cx: f32, cy: f32, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: rect(cx - radius, cy - radius, cx + radius, cy + radius),
        radiusX: radius,
        radiusY: radius,
    }
}

/// 自定义消息：请求回到完整模式。按钮和 Esc 都发它，处理逻辑只有一份。
const WM_RETURN_TO_FULL: u32 = WM_APP + 1;

/// 托盘发来的播放控制。走消息而不是直接调方法，是因为 `Ui` 只属于窗口线程。
const WM_TRAY_PLAY_PAUSE: u32 = WM_APP + 2;
const WM_TRAY_PREV: u32 = WM_APP + 3;
const WM_TRAY_NEXT: u32 = WM_APP + 4;

fn rgb(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

const COLOR_BG: u32 = 0x0f1115;
const COLOR_TEXT: u32 = 0xe6e8ee;
const COLOR_DIM: u32 = 0x8b90a0;
const COLOR_ACCENT: u32 = 0x6ea8fe;
const COLOR_LINE: u32 = 0x232633;
const COLOR_PANEL: u32 = 0x1a1d27;

/// 全局只允许一个迷你窗口。存 HWND 的裸值，`close()` 从别的线程也能给它发消息。
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static OPEN: AtomicBool = AtomicBool::new(false);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::Acquire)
}

/// 出了这个作用域就把「窗口开着」的状态清干净，正常返回和 panic 都算。
///
/// 不能只在 `pump()` 之后手动清：那底下是一千多行 unsafe 的 Direct2D / WinRT 调用，
/// 一次 panic 就会让 OPEN 永远停在 true。之后托盘的「退出」和「切换轻量模式」都会
/// 走进"轻量模式还开着"那条分支，对着一个已经没了的窗口 PostMessage，两个菜单项
/// 从此都没有任何反应，「显示/隐藏」也只会去 focus 那个不存在的窗口。
struct OpenGuard;

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::Release);
        WINDOW.store(0, Ordering::Release);
    }
}

/// 请求关窗。窗口不在就什么都不做。真正的清理在窗口线程里做。
pub fn close() {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(raw as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// 把窗口拉到前面。轻量模式下托盘点击只能做这件事——没有 WebView 可显示了。
pub fn focus() {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw != 0 {
        unsafe {
            let hwnd = HWND(raw as *mut c_void);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// 托盘菜单里的播放控制。轻量模式下 JS 那条路没了，只能发窗口消息，
/// 而且必须发到窗口线程去执行——`Ui` 不是线程安全的。
pub fn transport(action: LiteAction) {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    let msg = match action {
        LiteAction::PlayPause => WM_TRAY_PLAY_PAUSE,
        LiteAction::Prev => WM_TRAY_PREV,
        LiteAction::Next => WM_TRAY_NEXT,
    };
    unsafe {
        let _ = PostMessageW(Some(HWND(raw as *mut c_void)), msg, WPARAM(0), LPARAM(0));
    }
}

/// 打开迷你窗口。已经开着就把它拉到前面，不会开出第二个。
///
/// 窗口创建成功与否通过 channel 回报，避免调用方拿到一个"看起来成功了"的空壳。
pub fn open<F>(snapshot: MiniSnapshot, on_return_to_full: F) -> Result<(), String>
where
    F: Fn(MiniSnapshot) + Send + 'static,
{
    if OPEN.load(Ordering::Acquire) {
        let raw = WINDOW.load(Ordering::Acquire);
        if raw != 0 {
            unsafe {
                let hwnd = HWND(raw as *mut c_void);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        return Ok(());
    }

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("aura-mini".to_string())
        .spawn(move || unsafe {
            // STA：WIC 和 MediaPlayer 在单线程套间里都工作正常，窗口消息也要求单线程。
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            match Ui::create(snapshot, Box::new(on_return_to_full)) {
                Ok(mut ui) => {
                    let hwnd = ui.hwnd;
                    WINDOW.store(hwnd.0 as isize, Ordering::Release);
                    OPEN.store(true, Ordering::Release);
                    let _open_guard = OpenGuard;
                    let _ = tx.send(Ok(()));
                    ui.pump();
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            }
            CoUninitialize();
        })
        .map_err(|err| format!("启动轻量模式窗口线程失败: {err}"))?;

    rx.recv()
        .unwrap_or_else(|_| Err("轻量模式窗口线程没有回报状态".to_string()))
}

struct Brushes {
    text: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    line: ID2D1SolidColorBrush,
    panel: ID2D1SolidColorBrush,
}

struct Formats {
    title: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    time: IDWriteTextFormat,
    lyric_active: IDWriteTextFormat,
    lyric_dim: IDWriteTextFormat,
    translation: IDWriteTextFormat,
    queue: IDWriteTextFormat,
    /// 图标字体的三档字号。字体不存在时是 None，绘制走几何回退。
    icon_small: Option<IDWriteTextFormat>,
    icon_medium: Option<IDWriteTextFormat>,
    icon_large: Option<IDWriteTextFormat>,
}

unsafe fn font_exists(dwrite: &IDWriteFactory, family: &[u16]) -> bool {
    let mut collection: Option<windows::Win32::Graphics::DirectWrite::IDWriteFontCollection> = None;
    if dwrite
        .GetSystemFontCollection(&mut collection, false)
        .is_err()
    {
        return false;
    }
    let Some(collection) = collection else {
        return false;
    };
    let mut index = 0u32;
    let mut exists = BOOL(0);
    if collection
        .FindFamilyName(PCWSTR(family.as_ptr()), &mut index, &mut exists)
        .is_err()
    {
        return false;
    }
    exists.as_bool()
}

unsafe fn make_format(
    dwrite: &IDWriteFactory,
    family: &[u16],
    size: f32,
    semibold: bool,
    alignment: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
) -> windows::core::Result<IDWriteTextFormat> {
    let weight = if semibold {
        DWRITE_FONT_WEIGHT_SEMI_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    let locale = wide_z("");
    let format = dwrite.CreateTextFormat(
        PCWSTR(family.as_ptr()),
        None,
        weight,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        size,
        PCWSTR(locale.as_ptr()),
    )?;
    format.SetTextAlignment(alignment)?;
    format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    Ok(format)
}

/// 给会超宽的文本（标题、队列行）挂上省略号裁剪，否则长标题会直接画出边界。
unsafe fn add_ellipsis(
    dwrite: &IDWriteFactory,
    format: &IDWriteTextFormat,
) -> windows::core::Result<()> {
    let sign = dwrite.CreateEllipsisTrimmingSign(format)?;
    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    format.SetTrimming(&trimming, &sign)
}

impl Formats {
    unsafe fn create(dwrite: &IDWriteFactory, scale: f32) -> windows::core::Result<Self> {
        let ui = wide_z("Segoe UI");
        let fluent = wide_z("Segoe Fluent Icons");
        let mdl2 = wide_z("Segoe MDL2 Assets");
        let icon_family = if font_exists(dwrite, &fluent) {
            Some(fluent)
        } else if font_exists(dwrite, &mdl2) {
            Some(mdl2)
        } else {
            None
        };

        let title = make_format(dwrite, &ui, 16.0 * scale, true, DWRITE_TEXT_ALIGNMENT_CENTER)?;
        add_ellipsis(dwrite, &title)?;
        let queue = make_format(dwrite, &ui, 12.0 * scale, false, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        add_ellipsis(dwrite, &queue)?;

        let icon = |size: f32| -> windows::core::Result<Option<IDWriteTextFormat>> {
            match icon_family.as_ref() {
                None => Ok(None),
                Some(family) => Ok(Some(make_format(
                    dwrite,
                    family,
                    size * scale,
                    false,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                )?)),
            }
        };

        Ok(Self {
            title,
            sub: make_format(dwrite, &ui, 12.0 * scale, false, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            time: make_format(dwrite, &ui, 11.0 * scale, false, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            lyric_active: make_format(
                dwrite,
                &ui,
                15.0 * scale,
                true,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?,
            lyric_dim: make_format(dwrite, &ui, 12.0 * scale, false, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            translation: make_format(
                dwrite,
                &ui,
                11.0 * scale,
                false,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?,
            queue,
            icon_small: icon(14.0)?,
            icon_medium: icon(18.0)?,
            icon_large: icon(20.0)?,
        })
    }
}

struct Ui {
    hwnd: HWND,
    engine: Box<dyn AudioBackend>,
    /// 系统媒体控制（Windows 是 SMTC）。原来这套是 WinRT MediaPlayer 免费带的，
    /// 自己解码之后必须自己接，否则媒体键和锁屏信息就没了。
    media: Box<dyn MediaControls>,
    /// 与平台无关的播放状态机。换曲、循环、洗牌、失败退避都在它里面，
    /// 这里只负责把它返回的 `Action` 兑现成对 `engine` 的调用。
    playback: Playback,
    on_return: Box<dyn Fn(MiniSnapshot)>,

    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    target: Option<ID2D1HwndRenderTarget>,
    brushes: Option<Brushes>,
    formats: Option<Formats>,

    layout: Layout,
    /// 歌词轨与它的滚动状态，同样是平台无关的。
    lyrics: LyricTrack,
    cover: Option<ID2D1Bitmap>,
    cover_key: Option<String>,

    drag: Drag,
    queue_scroll: f32,

    last_paint: PaintKey,
}

static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

unsafe fn register_class() -> windows::core::Result<PCWSTR> {
    let name = CLASS_NAME.get_or_init(|| {
        let name = wide_z("AuraMiniPlayerWindow");
        let instance: HINSTANCE = GetModuleHandleW(None).map(|module| module.into()).unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        // 注册失败（比如同名类已存在）不致命：CreateWindowExW 会给出真正的错误。
        RegisterClassExW(&class);
        name
    });
    Ok(PCWSTR(name.as_ptr()))
}

impl Ui {
    unsafe fn create(
        snapshot: MiniSnapshot,
        on_return: Box<dyn Fn(MiniSnapshot)>,
    ) -> Result<Box<Self>, String> {
        // 快照的规范化由 Playback::new 负责，这里不必再来一遍。
        let class = register_class().map_err(|err| format!("注册窗口类失败: {err}"))?;

        // 先按 96 DPI 摆一个位置，窗口出来后再按真实 DPI 重算尺寸。
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut c_void),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let width = LOGICAL_WIDTH as i32;
        let height = LOGICAL_HEIGHT as i32;
        let x = work.left + ((work.right - work.left) - width).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - height).max(0) / 2;

        let title = wide_z("Aura 轻量模式");
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_MINIMIZEBOX,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default()),
            None,
        )
        .map_err(|err| format!("创建轻量模式窗口失败: {err}"))?;

        // 圆角是锦上添花，Win10 早期版本不支持这个属性，失败就算了。
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const c_void,
            std::mem::size_of_val(&pref) as u32,
        );

        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
            .map_err(|err| format!("创建 Direct2D 工厂失败: {err}"))?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
            .map_err(|err| format!("创建 DirectWrite 工厂失败: {err}"))?;
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|err| format!("创建 WIC 工厂失败: {err}"))?;
        // 地址解析：`file://` 直接摊成路径，远端直链先落盘再播。
        //
        // rodio 走的 symphonia 需要 `Read + Seek`，喂不了 http；而轻量模式本来就会给
        // 当前及后续若干首预缓存，走到这里下载的只是预缓存窗口之外的曲目。宁可等它落盘，
        // 也不要自己写一个可 seek 的 HTTP 读取器——那正是 proxy.rs 刚修掉的那类坑。
        let resolve: UriResolver = std::sync::Arc::new(|uri: &str, cache_id: &str| {
            super::resolve_playable_file(uri, cache_id)
        });
        let engine = Box::new(RodioBackend::new(resolve));
        let media: Box<dyn MediaControls> = match super::smtc::Smtc::new(hwnd) {
            Ok(smtc) => Box::new(smtc),
            Err(err) => {
                // 接不上系统媒体控制不影响放歌，降级成什么都不做，别把窗口带崩。
                eprintln!("[aura] SMTC 初始化失败，媒体键不可用: {err}");
                Box::new(super::shared::NoopMediaControls)
            }
        };

        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|delta| delta.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;

        let mut ui = Box::new(Self {
            hwnd,
            engine,
            media,
            playback: Playback::new(snapshot, seed),
            on_return,
            d2d,
            dwrite,
            wic,
            target: None,
            brushes: None,
            formats: None,
            layout: Layout::compute(scale),
            lyrics: LyricTrack::new(),
            cover: None,
            cover_key: None,
            drag: Drag::None,
            queue_scroll: 0.0,
            last_paint: PaintKey::default(),
        });

        // wndproc 通过 GWLP_USERDATA 找回这个对象。Box 的地址在移动 Box 时不变，
        // 所以这里存的指针在整个窗口生命周期内都有效。
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ui.as_mut() as *mut Ui as isize);
        ui.apply_scale(scale);
        ui.begin_current(true);
        SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        Ok(ui)
    }

    /// 按 DPI 缩放整窗。字体尺寸带在 `Formats` 里，所以缩放变了要连字体一起重建。
    unsafe fn apply_scale(&mut self, scale: f32) {
        self.layout = Layout::compute(scale);
        self.formats = None;
        let _ = SetWindowPos(
            self.hwnd,
            None,
            0,
            0,
            self.layout.width.round() as i32,
            self.layout.height.round() as i32,
            SWP_NOZORDER | SWP_NOMOVE,
        );
    }

    /// 建渲染目标、画刷和字体。设备可能在休眠、切换显卡、远程桌面之后丢掉，
    /// 所以这套东西必须能反复重建，不能只在启动时建一次。
    unsafe fn ensure_resources(&mut self) -> bool {
        if self.target.is_none() {
            let mut client = RECT::default();
            if GetClientRect(self.hwnd, &mut client).is_err() {
                return false;
            }
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                // DPI 钉在 96：于是 DIP 就是设备像素，缩放只有 Layout 一处在算。
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U {
                    width: (client.right - client.left).max(1) as u32,
                    height: (client.bottom - client.top).max(1) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            match self.d2d.CreateHwndRenderTarget(&props, &hwnd_props) {
                Ok(target) => {
                    target.SetDpi(96.0, 96.0);
                    self.target = Some(target);
                }
                Err(_) => return false,
            }
            // 画刷和位图都绑在旧目标上，跟着一起作废。
            self.brushes = None;
            self.cover = None;
            self.cover_key = None;
        }

        let Some(target) = self.target.clone() else {
            return false;
        };
        if self.brushes.is_none() {
            let solid = |hex: u32| target.CreateSolidColorBrush(&rgb(hex), None);
            match (
                solid(COLOR_TEXT),
                solid(COLOR_DIM),
                solid(COLOR_ACCENT),
                solid(COLOR_LINE),
                solid(COLOR_PANEL),
            ) {
                (Ok(text), Ok(dim), Ok(accent), Ok(line), Ok(panel)) => {
                    self.brushes = Some(Brushes {
                        text,
                        dim,
                        accent,
                        line,
                        panel,
                    });
                }
                _ => return false,
            }
        }
        if self.formats.is_none() {
            match Formats::create(&self.dwrite, self.layout.scale) {
                Ok(formats) => self.formats = Some(formats),
                Err(_) => return false,
            }
        }
        true
    }

    /// 设备丢失后整套丢掉，下一帧重建。
    fn discard_resources(&mut self) {
        self.target = None;
        self.brushes = None;
        self.cover = None;
        self.cover_key = None;
    }

    unsafe fn pump(&mut self) {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    /// 把状态机给出的动作兑现成对 `engine` 的调用。
    ///
    /// 装载失败会让状态机再吐出一批动作（往后跳，或者停手），所以这里用工作队列而不是
    /// 直接递归。失败计数已经把深度限制在三以内，但队列写法更不容易踩到栈上去。
    unsafe fn apply(&mut self, actions: Vec<Action>) {
        let mut queue: std::collections::VecDeque<Action> = actions.into();
        while let Some(action) = queue.pop_front() {
            match action {
                Action::SetVolume(volume) => {
                    let _ = self.engine.set_volume(volume);
                }
                Action::SetMuted(muted) => {
                    let _ = self.engine.set_muted(muted);
                }
                Action::Play => {
                    let _ = self.engine.play();
                    self.media.set_playing(true);
                }
                Action::Pause => {
                    let _ = self.engine.pause();
                    self.media.set_playing(false);
                }
                Action::Halt(_) => {
                    // 提示文案已经记在状态机里。停手就意味着后面排着的动作都作废。
                    queue.clear();
                    // 系统面板也别停在"正在播放"上。
                    self.media.set_playing(false);
                }
                Action::Load { index } => {
                    self.lyrics.reset();
                    // 地址解析要查磁盘缓存，所以留在这一侧做，状态机不碰文件系统。
                    let track = self
                        .playback
                        .snapshot()
                        .tracks
                        .get(index.max(0) as usize)
                        .cloned();
                    let loaded = match track.as_ref().and_then(|t| {
                        super::playable_uri(t).map(|uri| (uri, t.cache_id()))
                    }) {
                        Some((uri, cache_id)) => self.engine.load(&uri, &cache_id).is_ok(),
                        None => false,
                    };
                    if loaded {
                        // 系统媒体面板跟着换曲刷新。封面用快照里那份本地文件——
                        // 原生窗口没有 WebView 的图片加载能力，远端地址喂不进去。
                        if let Some(track) = track.as_ref() {
                            self.media.set_now_playing(&NowPlaying {
                                title: &track.title,
                                artist: &track.artist,
                                album: &track.album,
                                cover_path: track.cover_path.as_deref(),
                            });
                        }
                        self.playback.on_load_ok();
                    } else {
                        // 这一首废了，后面排着的 Play 也就没有意义了。
                        queue.clear();
                        queue.extend(self.playback.on_load_failed());
                    }
                }
            }
        }
    }

    /// 开始播放当前曲目。`resume` 表示这是刚从完整模式接管，要把进度接回去。
    unsafe fn begin_current(&mut self, resume: bool) {
        let actions = self.playback.begin_current(resume);
        self.apply(actions);
    }

    /// 换曲。`user` 为真表示这是用户点的，会清掉失败计数，也不走随机。
    unsafe fn step(&mut self, delta: i64, user: bool) {
        let actions = self.playback.step(delta, user);
        if actions.is_empty() {
            return;
        }
        // 换曲之后队列滚回顶部，不然新的当前曲目可能在可视区外。
        self.queue_scroll = 0.0;
        self.apply(actions);
    }

    unsafe fn jump_to(&mut self, index: i64) {
        let actions = self.playback.jump_to(index);
        if actions.is_empty() {
            return;
        }
        self.apply(actions);
    }

    unsafe fn toggle_play(&mut self) {
        let actions = self.playback.toggle_play(self.engine.is_playing());
        self.apply(actions);
    }

    fn paint_key(&self) -> PaintKey {
        PaintKey {
            quarter: PaintKey::quantize_seconds(self.playback.display_position()),
            duration: PaintKey::quantize_seconds(self.engine.duration()),
            playing: self.engine.is_playing(),
            busy: self.engine.is_busy(),
            index: self.playback.index(),
            lyric: self.lyrics.active(),
            volume: PaintKey::quantize_volume(self.playback.volume()),
            muted: self.playback.muted(),
            loop_mode: self.playback.loop_mode(),
            scroll: self.queue_scroll.round() as i32,
            sliding: self.lyrics.is_sliding(),
            notice: self.playback.notice().is_some(),
        }
    }

    unsafe fn on_tick(&mut self) {
        // 自动换曲这两条路（播完下一首、加载失败往后跳）原来都经过 Ui::step，顺带就把
        // 队列滚回顶部了；抽出 Playback 之后它们直接 apply，滚动位置留在原处——用户停在
        // 队列中段时自动换曲，当前曲目的高亮会跑到可视区外。
        //
        // 判据用「下标是否真的变了」而不是「有没有产生动作」：连挂三首停手那条会返回
        // Halt 但并不换曲，那种情况原来也不重置。
        let index_before = self.playback.index();
        if self.engine.take_failed() {
            let actions = self.playback.on_load_failed();
            self.apply(actions);
        } else if self.engine.take_ended() {
            let actions = self.playback.on_ended();
            self.apply(actions);
        }
        if self.playback.index() != index_before {
            self.queue_scroll = 0.0;
        }

        // 补上被丢掉的恢复 seek。一秒还没等到源打开就算了，从头播总比不播好。
        if let Some(target) = self
            .playback
            .take_pending_seek(self.engine.is_busy(), self.engine.duration())
        {
            let _ = self.engine.seek(target);
        }

        if self.drag != Drag::Progress {
            self.playback.set_position(self.engine.position());
        }

        let index = self.playback.index();
        let (lyric, tlyric) = match self.playback.snapshot().current() {
            Some(track) => (track.lyric.clone(), track.tlyric.clone()),
            None => (None, None),
        };
        self.lyrics.sync(index, lyric.as_deref(), tlyric.as_deref());
        self.lyrics
            .tick(self.playback.display_position(), self.layout.row_height);

        let key = self.paint_key();
        if key != self.last_paint || self.lyrics.is_sliding() {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    unsafe fn set_volume_from(&mut self, x: f32) {
        let ratio = self.layout.volume_slider.ratio_at(x);
        let actions = self.playback.set_volume_from_ratio(ratio);
        self.apply(actions);
    }

    unsafe fn adjust_volume(&mut self, delta: f64) {
        let actions = self.playback.adjust_volume(delta);
        self.apply(actions);
    }

    unsafe fn on_left_down(&mut self, x: f32, y: f32) {
        let layout = self.layout;
        // 关闭和"回到完整模式"走同一条出口：这个窗口一旦没了，整个应用就没界面了，
        // 所以关窗只能是把完整界面拉回来，不是退进程。
        if layout.btn_close.contains(x, y) {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        } else if layout.btn_full.contains(x, y) {
            let _ = PostMessageW(Some(self.hwnd), WM_RETURN_TO_FULL, WPARAM(0), LPARAM(0));
        } else if layout.btn_min.contains(x, y) {
            let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
        } else if layout.btn_play.contains(x, y) {
            self.toggle_play();
        } else if layout.btn_prev.contains(x, y) {
            self.step(-1, true);
        } else if layout.btn_next.contains(x, y) {
            self.step(1, true);
        } else if layout.btn_loop.contains(x, y) {
            self.playback.cycle_loop_mode();
        } else if layout.volume_icon.contains(x, y) {
            let actions = self.playback.toggle_muted();
            self.apply(actions);
        } else if layout.volume_hit.contains(x, y) {
            self.drag = Drag::Volume;
            SetCapture(self.hwnd);
            self.set_volume_from(x);
        } else if layout.progress_hit.contains(x, y) {
            // 拖的过程只改本地预览值，松手才真的 seek，免得一路拖一路重新缓冲。
            self.drag = Drag::Progress;
            SetCapture(self.hwnd);
            let target = layout.position_from(x, self.engine.duration());
            self.playback.set_scrub(Some(target));
        } else if layout.queue.contains(x, y) {
            if let Some(index) = layout.queue_index_at(y, self.queue_scroll, self.playback.count()) {
                if index != self.playback.index() {
                    self.jump_to(index);
                }
            }
        } else {
            return;
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn on_mouse_move(&mut self, x: f32) {
        match self.drag {
            Drag::Progress => {
                let target = self.layout.position_from(x, self.engine.duration());
                self.playback.set_scrub(Some(target));
            }
            Drag::Volume => self.set_volume_from(x),
            Drag::None => return,
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn on_left_up(&mut self) {
        if self.drag == Drag::None {
            return;
        }
        if self.drag == Drag::Progress {
            if let Some(target) = self.playback.scrub() {
                self.playback.set_scrub(None);
                let _ = self.engine.seek(target);
                self.playback.set_position(target);
            }
        }
        let _ = ReleaseCapture();
        self.drag = Drag::None;
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn on_wheel(&mut self, delta: i16, x: f32, y: f32) {
        let notches = delta as f32 / 120.0;
        if self.layout.queue.contains(x, y) {
            let row = self.layout.row_height;
            // 队列可视行数要扣掉顶上那行标题。
            let visible = ((self.layout.queue.height() / row) - 1.0).max(1.0);
            let max = (self.playback.count() as f32 - visible).max(0.0) * row;
            self.queue_scroll = (self.queue_scroll - notches * row * 2.0).clamp(0.0, max);
        } else {
            // 队列以外滚滚轮当调音量，和大多数播放器的习惯一致。
            self.adjust_volume(notches as f64 * 0.05);
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn nudge(&mut self, delta: f64) {
        let duration = self.engine.duration();
        if duration <= 0.0 {
            return;
        }
        let target = (self.engine.position() + delta).clamp(0.0, (duration - 0.5).max(0.0));
        let _ = self.engine.seek(target);
        self.playback.set_position(target);
    }

    unsafe fn on_key(&mut self, vk: u16) {
        // Ctrl + 左右是换曲，光秃秃的左右是快退快进。
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        if vk == VK_SPACE.0 {
            self.toggle_play();
        } else if vk == VK_ESCAPE.0 {
            let _ = PostMessageW(Some(self.hwnd), WM_RETURN_TO_FULL, WPARAM(0), LPARAM(0));
            return;
        } else if vk == VK_LEFT.0 {
            if ctrl {
                self.step(-1, true);
            } else {
                self.nudge(-5.0);
            }
        } else if vk == VK_RIGHT.0 {
            if ctrl {
                self.step(1, true);
            } else {
                self.nudge(5.0);
            }
        } else if vk == VK_UP.0 {
            self.adjust_volume(0.05);
        } else if vk == VK_DOWN.0 {
            self.adjust_volume(-0.05);
        } else {
            return;
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    /// 退出前把状态交回去：快照落盘 + 回调，完整模式接着从这个位置往下播。
    unsafe fn finish(&mut self) {
        if self.engine.duration() > 0.0 {
            self.playback.set_position(self.engine.position());
        }
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|delta| delta.as_secs() as i64)
            .unwrap_or(0);
        self.playback.snapshot_mut().saved_at = saved_at;
        let _ = self.engine.pause();
        // 收摊：别在系统媒体面板上留一条永远停在这首歌的记录。
        self.media.clear();
        let snapshot = self.playback.snapshot().clone();
        let _ = snapshot::save(&snapshot);
        (self.on_return)(snapshot);
    }

    unsafe fn text(
        target: &ID2D1HwndRenderTarget,
        content: &str,
        format: &IDWriteTextFormat,
        area: &D2D_RECT_F,
        brush: &ID2D1SolidColorBrush,
    ) {
        if content.is_empty() {
            return;
        }
        let buffer = wide(content);
        target.DrawText(
            &buffer,
            format,
            area,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }

    unsafe fn draw_icon(
        target: &ID2D1HwndRenderTarget,
        icon: Icon,
        area: &D2D_RECT_F,
        brush: &ID2D1SolidColorBrush,
        format: Option<&IDWriteTextFormat>,
    ) {
        if let Some(format) = format {
            let mut buffer = [0u16; 2];
            let encoded = icon.glyph().encode_utf16(&mut buffer);
            target.DrawText(
                encoded,
                format,
                area,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            return;
        }
        draw_fallback(target, icon.fallback(), area, brush);
    }

    /// 封面换了才重新解码。同一张图不该每帧都过一遍 WIC。
    unsafe fn ensure_cover(&mut self, target: &ID2D1HwndRenderTarget) {
        let path = self
            .playback
            .snapshot()
            .current()
            .and_then(|track| track.cover_path.clone())
            .unwrap_or_default();
        if self.cover_key.as_deref() == Some(path.as_str()) {
            return;
        }
        self.cover_key = Some(path.clone());
        self.cover = if path.is_empty() {
            None
        } else {
            self.load_cover(target, &path)
        };
    }

    /// WIC 解码封面。失败就当没有封面画占位图标——不该因为一张图挂掉整个界面。
    unsafe fn load_cover(
        &self,
        target: &ID2D1HwndRenderTarget,
        path: &str,
    ) -> Option<ID2D1Bitmap> {
        let file = wide_z(path);
        let decoder = self
            .wic
            .CreateDecoderFromFilename(
                PCWSTR(file.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let converter = self.wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;
        target.CreateBitmapFromWicBitmap(&converter, None).ok()
    }

    unsafe fn on_paint(&mut self) {
        let mut ps = PAINTSTRUCT::default();
        BeginPaint(self.hwnd, &mut ps);
        let lost = self.render();
        let _ = EndPaint(self.hwnd, &ps);
        // 设备丢了才丢资源，下一帧会整套重建。
        if lost {
            self.discard_resources();
        }
    }

    /// 返回 true 表示渲染目标已经失效。
    unsafe fn render(&mut self) -> bool {
        if !self.ensure_resources() {
            return false;
        }
        let Some(target) = self.target.clone() else {
            return false;
        };
        self.ensure_cover(&target);
        let key = self.paint_key();

        let Some(brushes) = self.brushes.as_ref() else {
            return false;
        };
        let Some(formats) = self.formats.as_ref() else {
            return false;
        };
        target.BeginDraw();
        target.Clear(Some(&rgb(COLOR_BG)));
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        self.draw_chrome(&target, brushes, formats);
        self.draw_cover(&target, brushes, formats);
        self.draw_meta(&target, brushes, formats);
        self.draw_progress(&target, brushes, formats);
        self.draw_controls(&target, brushes, formats);
        self.draw_lyrics(&target, brushes, formats);
        self.draw_queue(&target, brushes, formats);
        if let Err(err) = target.EndDraw(None, None) {
            return err.code() == D2DERR_RECREATE_TARGET;
        }
        self.last_paint = key;
        false
    }

    unsafe fn draw_chrome(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let label = rect(16.0 * l.scale, 8.0 * l.scale, l.btn_full.left, 30.0 * l.scale);
        Self::text(target, "Aura 轻量模式", &formats.time, &label, &brushes.line);
        let icons = formats.icon_small.as_ref();
        Self::draw_icon(target, Icon::FullMode, &d2d(l.btn_full), &brushes.dim, icons);
        Self::draw_icon(target, Icon::Minimize, &d2d(l.btn_min), &brushes.dim, icons);
        Self::draw_icon(target, Icon::Close, &d2d(l.btn_close), &brushes.dim, icons);
    }

    unsafe fn draw_cover(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let radius = 10.0 * l.scale;
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: d2d(l.cover),
                radiusX: radius,
                radiusY: radius,
            },
            &brushes.panel,
        );
        match self.cover.as_ref() {
            Some(bitmap) => {
                target.PushAxisAlignedClip(&d2d(l.cover), D2D1_ANTIALIAS_MODE_ALIASED);
                target.DrawBitmap(
                    bitmap,
                    Some(&d2d(l.cover)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                target.PopAxisAlignedClip();
            }
            None => Self::draw_icon(
                target,
                Icon::Note,
                &d2d(l.cover),
                &brushes.line,
                formats.icon_large.as_ref(),
            ),
        }
    }

    unsafe fn draw_meta(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let (title, artist, album) = match self.playback.snapshot().current() {
            Some(track) => (
                track.title.as_str(),
                track.artist.as_str(),
                track.album.as_str(),
            ),
            None => ("没有正在播放的曲目", "", ""),
        };
        Self::text(target, title, &formats.title, &d2d(l.title), &brushes.text);

        // 副标题这一行是复用的：有话要说的时候（失败、缓冲）优先说话。
        if let Some(notice) = self.playback.notice() {
            Self::text(target, notice, &formats.sub, &d2d(l.subtitle), &brushes.accent);
            return;
        }
        if self.engine.is_busy() {
            Self::text(target, "缓冲中…", &formats.sub, &d2d(l.subtitle), &brushes.dim);
            return;
        }
        let subtitle = if album.is_empty() {
            artist.to_string()
        } else {
            format!("{artist} · {album}")
        };
        Self::text(target, &subtitle, &formats.sub, &d2d(l.subtitle), &brushes.dim);
    }

    unsafe fn draw_progress(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let duration = self.engine.duration();
        let position = self.playback.display_position();
        let ratio = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let bar = l.progress;
        let radius = (bar.bottom - bar.top) / 2.0;
        let rounded = |area: D2D_RECT_F| D2D1_ROUNDED_RECT {
            rect: area,
            radiusX: radius,
            radiusY: radius,
        };
        target.FillRoundedRectangle(&rounded(d2d(bar)), &brushes.line);
        let head = bar.left + (bar.right - bar.left) * ratio;
        if ratio > 0.0 {
            target.FillRoundedRectangle(
                &rounded(rect(bar.left, bar.top, head, bar.bottom)),
                &brushes.accent,
            );
        }
        // 拖动时给个把手，不然手指不知道自己拖到哪了。
        if self.drag == Drag::Progress {
            target.FillRoundedRectangle(
                &circle(head, (bar.top + bar.bottom) / 2.0, 5.0 * l.scale),
                &brushes.text,
            );
        }
        let clock = format!("{} / {}", fmt_time(position), fmt_time(duration));
        Self::text(target, &clock, &formats.time, &d2d(l.time_row), &brushes.dim);
    }

    unsafe fn draw_controls(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let small = formats.icon_small.as_ref();
        let (loop_icon, loop_brush) = match self.playback.snapshot().loop_mode {
            1 => (Icon::LoopOne, &brushes.accent),
            2 => (Icon::Shuffle, &brushes.accent),
            _ => (Icon::LoopSequence, &brushes.dim),
        };
        Self::draw_icon(target, loop_icon, &d2d(l.btn_loop), loop_brush, small);
        Self::draw_icon(
            target,
            Icon::Prev,
            &d2d(l.btn_prev),
            &brushes.text,
            formats.icon_medium.as_ref(),
        );

        // 主按钮垫一个圆底，让它在一排图标里显眼。
        let play = l.btn_play;
        target.FillRoundedRectangle(
            &circle(
                (play.left + play.right) / 2.0,
                (play.top + play.bottom) / 2.0,
                (play.right - play.left) / 2.0,
            ),
            &brushes.panel,
        );
        let play_icon = if self.engine.is_playing() {
            Icon::Pause
        } else {
            Icon::Play
        };
        Self::draw_icon(
            target,
            play_icon,
            &d2d(play),
            &brushes.text,
            formats.icon_large.as_ref(),
        );
        Self::draw_icon(
            target,
            Icon::Next,
            &d2d(l.btn_next),
            &brushes.text,
            formats.icon_medium.as_ref(),
        );

        let silent = self.playback.snapshot().muted || self.playback.snapshot().volume <= 0.0;
        let volume_icon = if silent { Icon::Muted } else { Icon::Volume };
        Self::draw_icon(target, volume_icon, &d2d(l.volume_icon), &brushes.dim, small);
        let slider = l.volume_slider;
        let radius = (slider.bottom - slider.top) / 2.0;
        let rounded = |area: D2D_RECT_F| D2D1_ROUNDED_RECT {
            rect: area,
            radiusX: radius,
            radiusY: radius,
        };
        target.FillRoundedRectangle(&rounded(d2d(slider)), &brushes.line);
        let level = if silent {
            0.0
        } else {
            self.playback.snapshot().volume as f32
        };
        if level > 0.0 {
            let head = slider.left + (slider.right - slider.left) * level;
            target.FillRoundedRectangle(
                &rounded(rect(slider.left, slider.top, head, slider.bottom)),
                &brushes.text,
            );
        }
    }

    unsafe fn draw_lyrics(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let lines = self.lyrics.lines();
        let active_line = self.lyrics.active();
        if lines.is_empty() {
            Self::text(
                target,
                "没有歌词",
                &formats.lyric_dim,
                &d2d(l.lyrics),
                &brushes.line,
            );
            return;
        }
        target.PushAxisAlignedClip(&d2d(l.lyrics), D2D1_ANTIALIAS_MODE_ALIASED);
        let row = l.row_height;
        let center = (l.lyrics.top + l.lyrics.bottom) / 2.0;
        let total = lines.len() as i64;
        let translated = lines
            .get(active_line.max(0) as usize)
            .map(|line| line.translation.is_some())
            .unwrap_or(false);

        for offset in -2i64..=2 {
            let index = active_line + offset;
            if index < 0 || index >= total {
                continue;
            }
            // 当前行有译文时，译文占掉了下一行的位置。
            if offset == 1 && translated && active_line >= 0 {
                continue;
            }
            let line = &lines[index as usize];
            let top = center + offset as f32 * row - row / 2.0 + self.lyrics.shift();
            let area = rect(l.lyrics.left, top, l.lyrics.right, top + row);
            if offset == 0 && active_line >= 0 {
                Self::text(target, &line.text, &formats.lyric_active, &area, &brushes.text);
                if let Some(translation) = line.translation.as_deref() {
                    let below = rect(l.lyrics.left, top + row, l.lyrics.right, top + row * 2.0);
                    Self::text(
                        target,
                        translation,
                        &formats.translation,
                        &below,
                        &brushes.accent,
                    );
                }
            } else {
                Self::text(target, &line.text, &formats.lyric_dim, &area, &brushes.dim);
            }
        }
        target.PopAxisAlignedClip();
    }

    unsafe fn draw_queue(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let radius = 8.0 * l.scale;
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: d2d(l.queue),
                radiusX: radius,
                radiusY: radius,
            },
            &brushes.panel,
        );
        let row = l.row_height;
        let pad = 12.0 * l.scale;
        let header = rect(
            l.queue.left + pad,
            l.queue.top,
            l.queue.right - pad,
            l.queue.top + row,
        );
        let count = self.playback.snapshot().tracks.len();
        Self::text(
            target,
            &format!("播放队列 · {count} 首"),
            &formats.time,
            &header,
            &brushes.dim,
        );

        let body_top = l.queue.top + row;
        target.PushAxisAlignedClip(
            &rect(l.queue.left, body_top, l.queue.right, l.queue.bottom),
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        let mut top = body_top - self.queue_scroll;
        for (index, track) in self.playback.snapshot().tracks.iter().enumerate() {
            let bottom = top + row;
            if bottom > body_top && top < l.queue.bottom {
                let current = index as i64 == self.playback.snapshot().index;
                if current {
                    target.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: rect(
                                l.queue.left + pad * 0.5,
                                top,
                                l.queue.right - pad * 0.5,
                                bottom,
                            ),
                            radiusX: 6.0 * l.scale,
                            radiusY: 6.0 * l.scale,
                        },
                        &brushes.line,
                    );
                }
                let label = format!("{}. {} — {}", index + 1, track.title, track.artist);
                let brush = if current { &brushes.text } else { &brushes.dim };
                let area = rect(l.queue.left + pad, top, l.queue.right - pad, bottom);
                Self::text(target, &label, &formats.queue, &area, brush);
            }
            top = bottom;
            if top > l.queue.bottom {
                break;
            }
        }
        target.PopAxisAlignedClip();
    }
}

unsafe fn draw_fallback(
    target: &ID2D1HwndRenderTarget,
    shape: Fallback,
    area: &D2D_RECT_F,
    brush: &ID2D1SolidColorBrush,
) {
    let cx = (area.left + area.right) / 2.0;
    let cy = (area.top + area.bottom) / 2.0;
    let half = (area.right - area.left).min(area.bottom - area.top) / 2.0 * 0.55;
    if half <= 0.0 {
        return;
    }
    // 三角形用竖条堆出来，X 用小方块沿对角线堆出来。
    let steps = 6usize;
    let unit = half * 2.0 / steps as f32;
    match shape {
        Fallback::TriangleRight | Fallback::TriangleLeft => {
            for step in 0..steps {
                let t = step as f32 / steps as f32;
                let height = half * (1.0 - t);
                let left = if matches!(shape, Fallback::TriangleRight) {
                    cx - half + step as f32 * unit
                } else {
                    cx + half - (step as f32 + 1.0) * unit
                };
                target.FillRectangle(
                    &rect(left, cy - height, left + unit, cy + height),
                    brush,
                );
            }
        }
        Fallback::TwoBars => {
            let width = half * 0.42;
            target.FillRectangle(
                &rect(cx - half * 0.7, cy - half, cx - half * 0.7 + width, cy + half),
                brush,
            );
            target.FillRectangle(
                &rect(cx + half * 0.7 - width, cy - half, cx + half * 0.7, cy + half),
                brush,
            );
        }
        Fallback::Bar => {
            target.FillRectangle(&rect(cx - half, cy - unit / 2.0, cx + half, cy + unit / 2.0), brush);
        }
        Fallback::Cross => {
            for step in 0..steps {
                let t = step as f32 * unit;
                target.FillRectangle(
                    &rect(cx - half + t, cy - half + t, cx - half + t + unit, cy - half + t + unit),
                    brush,
                );
                target.FillRectangle(
                    &rect(cx - half + t, cy + half - t - unit, cx - half + t + unit, cy + half - t),
                    brush,
                );
            }
        }
        Fallback::Frame => {
            let thickness = unit / 2.0;
            target.FillRectangle(&rect(cx - half, cy - half, cx + half, cy - half + thickness), brush);
            target.FillRectangle(&rect(cx - half, cy + half - thickness, cx + half, cy + half), brush);
            target.FillRectangle(&rect(cx - half, cy - half, cx - half + thickness, cy + half), brush);
            target.FillRectangle(&rect(cx + half - thickness, cy - half, cx + half, cy + half), brush);
        }
        Fallback::Dot => {
            target.FillRoundedRectangle(&circle(cx, cy, half * 0.5), brush);
        }
    }
}

/// lParam 里打包的是一对有符号 16 位坐标，负值（鼠标划出窗口）必须保住符号。
fn client_point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    (x as f32, y as f32)
}

/// `WM_MOUSEWHEEL` 和 `WM_NCHITTEST` 给的是屏幕坐标，得换回客户区。
unsafe fn screen_point(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let (x, y) = client_point(lparam);
    let mut point = POINT {
        x: x as i32,
        y: y as i32,
    };
    let _ = ScreenToClient(hwnd, &mut point);
    (point.x as f32, point.y as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
    if raw.is_null() {
        // CreateWindowExW 在 USERDATA 写进去之前就会发几条消息，这时候没什么可做的。
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ui = &mut *raw;

    match msg {
        WM_PAINT => {
            ui.on_paint();
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_ID {
                ui.on_tick();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(target) = ui.target.as_ref() {
                let size = D2D_SIZE_U {
                    width: (lparam.0 & 0xFFFF) as u32,
                    height: ((lparam.0 >> 16) & 0xFFFF) as u32,
                };
                if size.width > 0 && size.height > 0 {
                    let _ = target.Resize(&size);
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            ui.apply_scale(if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 });
            ui.discard_resources();
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 标题栏的空白处报成标题栏，拖窗、贴边、双击最大化全是系统白送的。
            let (x, y) = screen_point(hwnd, lparam);
            let l = &ui.layout;
            if l.title_bar.contains(x, y)
                && !l.btn_full.contains(x, y)
                && !l.btn_min.contains(x, y)
                && !l.btn_close.contains(x, y)
            {
                return LRESULT(HTCAPTION as isize);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            ui.on_left_down(x, y);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, _) = client_point(lparam);
            ui.on_mouse_move(x);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            ui.on_left_up();
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            let (x, y) = screen_point(hwnd, lparam);
            ui.on_wheel(delta, x, y);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            ui.on_key((wparam.0 & 0xFFFF) as u16);
            LRESULT(0)
        }
        WM_TRAY_PLAY_PAUSE => {
            ui.toggle_play();
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_TRAY_PREV => {
            ui.step(-1, true);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_TRAY_NEXT => {
            ui.step(1, true);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_RETURN_TO_FULL | WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_ID);
            ui.finish();
            // 先摘掉指针再退消息循环：之后进来的消息都会走 raw.is_null() 那条路。
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}









