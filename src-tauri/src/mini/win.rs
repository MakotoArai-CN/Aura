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

use super::lyrics::{self, LyricLine};
use super::snapshot::{self, MiniSnapshot};

/// 逻辑尺寸。窗口不可缩放，所有布局都按这套坐标写死，再按 DPI 整体缩放。
const LOGICAL_WIDTH: f32 = 420.0;
const LOGICAL_HEIGHT: f32 = 620.0;

const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 100;

/// 自定义消息：请求回到完整模式。按钮和 Esc 都发它，处理逻辑只有一份。
const WM_RETURN_TO_FULL: u32 = WM_APP + 1;

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

/// 请求关窗。窗口不在就什么都不做。真正的清理在窗口线程里做。
pub fn close() {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(raw as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
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
                    let _ = tx.send(Ok(()));
                    ui.pump();
                    OPEN.store(false, Ordering::Release);
                    WINDOW.store(0, Ordering::Release);
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

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn contains(area: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= area.left && x < area.right && y >= area.top && y < area.bottom
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

/// 一次算好、绘制和命中测试共用的布局。两边读同一份数据，就不可能算出两套坐标。
#[derive(Clone, Copy, Default)]
struct Layout {
    scale: f32,
    width: f32,
    height: f32,
    title_bar: D2D_RECT_F,
    btn_full: D2D_RECT_F,
    btn_min: D2D_RECT_F,
    btn_close: D2D_RECT_F,
    cover: D2D_RECT_F,
    title: D2D_RECT_F,
    subtitle: D2D_RECT_F,
    progress: D2D_RECT_F,
    progress_hit: D2D_RECT_F,
    time_row: D2D_RECT_F,
    btn_loop: D2D_RECT_F,
    btn_prev: D2D_RECT_F,
    btn_play: D2D_RECT_F,
    btn_next: D2D_RECT_F,
    volume_icon: D2D_RECT_F,
    volume_slider: D2D_RECT_F,
    volume_hit: D2D_RECT_F,
    lyrics: D2D_RECT_F,
    queue: D2D_RECT_F,
    row_height: f32,
}

impl Layout {
    fn compute(scale: f32) -> Self {
        let s = |value: f32| value * scale;
        Self {
            scale,
            width: s(LOGICAL_WIDTH),
            height: s(LOGICAL_HEIGHT),
            title_bar: rect(0.0, 0.0, s(420.0), s(36.0)),
            btn_full: rect(s(312.0), s(4.0), s(340.0), s(32.0)),
            btn_min: rect(s(348.0), s(4.0), s(376.0), s(32.0)),
            btn_close: rect(s(384.0), s(4.0), s(412.0), s(32.0)),
            cover: rect(s(120.0), s(48.0), s(300.0), s(228.0)),
            title: rect(s(24.0), s(240.0), s(396.0), s(264.0)),
            subtitle: rect(s(24.0), s(266.0), s(396.0), s(286.0)),
            progress: rect(s(24.0), s(304.0), s(396.0), s(308.0)),
            progress_hit: rect(s(24.0), s(296.0), s(396.0), s(316.0)),
            time_row: rect(s(24.0), s(314.0), s(396.0), s(330.0)),
            btn_loop: rect(s(24.0), s(340.0), s(48.0), s(364.0)),
            btn_prev: rect(s(136.0), s(336.0), s(168.0), s(368.0)),
            btn_play: rect(s(188.0), s(330.0), s(232.0), s(374.0)),
            btn_next: rect(s(252.0), s(336.0), s(284.0), s(368.0)),
            volume_icon: rect(s(312.0), s(343.0), s(330.0), s(361.0)),
            volume_slider: rect(s(336.0), s(350.0), s(396.0), s(354.0)),
            volume_hit: rect(s(330.0), s(342.0), s(400.0), s(362.0)),
            lyrics: rect(s(24.0), s(390.0), s(396.0), s(510.0)),
            queue: rect(s(12.0), s(520.0), s(408.0), s(612.0)),
            row_height: s(26.0),
        }
    }
}

/// 需要画的图标。有图标字体就用字形，没有就退回几何图形——宁可简陋也不要豆腐块。
#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Prev,
    Next,
    Play,
    Pause,
    Minimize,
    Close,
    FullMode,
    LoopSequence,
    LoopOne,
    Shuffle,
    Volume,
    Muted,
    Note,
}

impl Icon {
    /// Segoe Fluent Icons / Segoe MDL2 Assets 里的私有区码位，两套字体这些码位一致。
    fn glyph(self) -> char {
        match self {
            Icon::Prev => '\u{E100}',
            Icon::Next => '\u{E101}',
            Icon::Play => '\u{E102}',
            Icon::Pause => '\u{E103}',
            Icon::Minimize => '\u{E921}',
            Icon::Close => '\u{E8BB}',
            Icon::FullMode => '\u{E740}',
            Icon::LoopSequence => '\u{E1CD}',
            Icon::LoopOne => '\u{E1CC}',
            Icon::Shuffle => '\u{E14B}',
            Icon::Volume => '\u{E767}',
            Icon::Muted => '\u{E74F}',
            Icon::Note => '\u{E8D6}',
        }
    }
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

/// 鼠标正在拖什么。窗口本身的拖动交给 `WM_NCHITTEST` 返回 `HTCAPTION`，
/// 那样能白拿系统的贴边和动画，不用自己算偏移。
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Progress,
    Volume,
}

/// 决定"这一帧和上一帧比有没有变化"。只有它变了才 `InvalidateRect`——
/// 轻量模式要是无条件每秒重画十次，那就白轻量了。
#[derive(Clone, Copy, PartialEq, Default)]
struct PaintKey {
    quarter: i64,
    duration: i64,
    playing: bool,
    busy: bool,
    index: i64,
    lyric: i64,
    volume: i64,
    muted: bool,
    loop_mode: u8,
    scroll: i32,
    sliding: bool,
    notice: bool,
}

struct Ui {
    hwnd: HWND,
    engine: super::audio::Engine,
    snapshot: MiniSnapshot,
    on_return: Box<dyn Fn(MiniSnapshot)>,

    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    target: Option<ID2D1HwndRenderTarget>,
    brushes: Option<Brushes>,
    formats: Option<Formats>,

    layout: Layout,
    lyrics: Vec<LyricLine>,
    lyrics_for: i64,
    cover: Option<ID2D1Bitmap>,
    cover_key: Option<String>,

    drag: Drag,
    /// 拖动进度条时先本地预览这个位置，松手才真的 seek，免得一路拖一路重新缓冲。
    scrub: Option<f64>,
    queue_scroll: f32,
    /// 换行时的滑动偏移，每帧向 0 收敛，让歌词是滑过去而不是跳过去。
    lyric_shift: f32,
    active_line: i64,

    /// 恢复进度：刚 load 完源还在打开中，这时 seek 会被丢掉，得等它开好。
    pending_seek: Option<(f64, u32)>,
    /// 连续失败计数。连挂 3 首以上就停下来，不再无脑往后跳。
    failures: u32,
    notice: Option<&'static str>,
    last_paint: PaintKey,
    /// 随机播放用的 xorshift 种子。为一个洗牌引一个 rand 依赖不值得。
    rng: u64,
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
        mut snapshot: MiniSnapshot,
        on_return: Box<dyn Fn(MiniSnapshot)>,
    ) -> Result<Box<Self>, String> {
        snapshot.normalize();
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
        let engine = super::audio::Engine::new()
            .map_err(|err| format!("初始化系统播放器失败: {err}"))?;

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
            snapshot,
            on_return,
            d2d,
            dwrite,
            wic,
            target: None,
            brushes: None,
            formats: None,
            layout: Layout::compute(scale),
            lyrics: Vec::new(),
            lyrics_for: -1,
            cover: None,
            cover_key: None,
            drag: Drag::None,
            scrub: None,
            queue_scroll: 0.0,
            lyric_shift: 0.0,
            active_line: -1,
            pending_seek: None,
            failures: 0,
            notice: None,
            last_paint: PaintKey::default(),
            rng: seed,
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

    /// 开始播放当前曲目。`resume` 表示这是刚从完整模式接管，要把进度接回去。
    unsafe fn begin_current(&mut self, resume: bool) {
        let _ = self.engine.set_volume(self.snapshot.volume);
        let _ = self.engine.set_muted(self.snapshot.muted);
        self.lyrics_for = -1;
        self.scrub = None;
        self.lyric_shift = 0.0;
        self.active_line = -1;
        self.pending_seek = None;

        let Some(track) = self.snapshot.current().cloned() else {
            self.notice = Some("播放列表是空的，回到完整模式选歌");
            return;
        };
        let Some(uri) = super::playable_uri(&track) else {
            self.on_track_failed();
            return;
        };
        if self.engine.load(&uri).is_err() {
            self.on_track_failed();
            return;
        }
        // 恢复进度不能马上 seek：源还在打开中，这时候的 seek 会被丢掉。
        if resume && self.snapshot.position > 0.5 {
            self.pending_seek = Some((self.snapshot.position, 0));
        }
        let _ = self.engine.play();
        self.notice = None;
    }

    /// 一首打不开就往后跳。连挂三首就停手——这种情况基本是签名直链集体过期，
    /// 再往后跳也是同样的结果，不如告诉用户回完整模式重新解析。
    unsafe fn on_track_failed(&mut self) {
        self.failures += 1;
        if self.failures >= 3 || self.snapshot.tracks.len() <= 1 {
            let _ = self.engine.pause();
            self.notice = Some("无可播放的曲目，回到完整模式重新解析");
            return;
        }
        self.step(1, false);
    }

    /// 换曲。`user` 为真表示这是用户点的，会清掉失败计数，也不走随机。
    unsafe fn step(&mut self, delta: i64, user: bool) {
        let count = self.snapshot.tracks.len() as i64;
        if count == 0 {
            return;
        }
        if user {
            self.failures = 0;
        }
        self.snapshot.index = if self.snapshot.loop_mode == 2 && !user && delta > 0 {
            self.random_index(count)
        } else {
            (self.snapshot.index + delta).rem_euclid(count)
        };
        self.snapshot.position = 0.0;
        self.queue_scroll = 0.0;
        self.begin_current(false);
    }

    /// xorshift64。为一个洗牌拉一个 rand 依赖不值当。
    fn random_index(&mut self, count: i64) -> i64 {
        if count <= 1 {
            return 0;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let mut pick = (self.rng % count as u64) as i64;
        // 随机播放至少得换一首，原地重放会被当成卡住了。
        if pick == self.snapshot.index {
            pick = (pick + 1) % count;
        }
        pick
    }

    unsafe fn toggle_play(&mut self) {
        // 上一轮失败停下之后，再点一次播放就当作重试。
        if self.notice.is_some() {
            self.failures = 0;
            self.notice = None;
            self.begin_current(false);
            return;
        }
        if self.engine.is_playing() {
            let _ = self.engine.pause();
        } else {
            let _ = self.engine.play();
        }
    }

    fn ensure_lyrics(&mut self) {
        if self.lyrics_for == self.snapshot.index {
            return;
        }
        self.lyrics_for = self.snapshot.index;
        self.lyrics = match self.snapshot.current() {
            Some(track) => {
                let main = lyrics::parse(track.lyric.as_deref().unwrap_or(""));
                match track.tlyric.as_deref() {
                    Some(raw) if !raw.trim().is_empty() => {
                        lyrics::merge_translation(main, &lyrics::parse(raw))
                    }
                    _ => main,
                }
            }
            None => Vec::new(),
        };
        self.active_line = -1;
    }

    /// 拖动进度条时显示手指的位置而不是引擎的位置，不然拖起来会来回跳。
    fn display_position(&self) -> f64 {
        self.scrub.unwrap_or(self.snapshot.position)
    }

    fn paint_key(&self) -> PaintKey {
        PaintKey {
            quarter: (self.display_position() * 4.0) as i64,
            duration: (self.engine.duration() * 4.0) as i64,
            playing: self.engine.is_playing(),
            busy: self.engine.is_busy(),
            index: self.snapshot.index,
            lyric: self.active_line,
            volume: (self.snapshot.volume * 100.0) as i64,
            muted: self.snapshot.muted,
            loop_mode: self.snapshot.loop_mode,
            scroll: self.queue_scroll.round() as i32,
            sliding: self.lyric_shift != 0.0,
            notice: self.notice.is_some(),
        }
    }

    unsafe fn on_tick(&mut self) {
        if self.engine.take_failed() {
            self.on_track_failed();
        } else if self.engine.take_ended() {
            self.failures = 0;
            if self.snapshot.loop_mode == 1 {
                let _ = self.engine.seek(0.0);
                let _ = self.engine.play();
            } else {
                self.step(1, false);
            }
        }

        // 补上被丢掉的恢复 seek。一秒还没等到源打开就算了，从头播总比不播好。
        if let Some((target, tries)) = self.pending_seek {
            if !self.engine.is_busy() && self.engine.duration() > 0.0 {
                let _ = self.engine.seek(target);
                self.pending_seek = None;
            } else if tries >= 10 {
                self.pending_seek = None;
            } else {
                self.pending_seek = Some((target, tries + 1));
            }
        }

        if self.drag != Drag::Progress {
            self.snapshot.position = self.engine.position();
        }
        self.ensure_lyrics();

        let active = lyrics::active_index(&self.lyrics, self.display_position())
            .map(|index| index as i64)
            .unwrap_or(-1);
        if active != self.active_line {
            // 从上一行的位置滑过来，而不是直接跳。
            if self.active_line >= 0 {
                let delta = (active - self.active_line) as f32;
                self.lyric_shift = (delta * self.layout.row_height).clamp(-120.0, 120.0);
            }
            self.active_line = active;
        }
        if self.lyric_shift.abs() > 0.5 {
            self.lyric_shift *= 0.68;
        } else {
            self.lyric_shift = 0.0;
        }

        let key = self.paint_key();
        if key != self.last_paint || self.lyric_shift != 0.0 {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// 横向命中位置换成 0~1 的比例。进度条和音量条共用。
    fn ratio_in(area: &D2D_RECT_F, x: f32) -> f64 {
        let width = area.right - area.left;
        if width <= 0.0 {
            return 0.0;
        }
        (((x - area.left) / width) as f64).clamp(0.0, 1.0)
    }

    unsafe fn set_volume_from(&mut self, x: f32) {
        self.snapshot.volume = Self::ratio_in(&self.layout.volume_slider, x);
        // 手动拖音量的意思就是"我要听"，顺手解除静音。
        self.snapshot.muted = false;
        let _ = self.engine.set_muted(false);
        let _ = self.engine.set_volume(self.snapshot.volume);
    }

    unsafe fn adjust_volume(&mut self, delta: f64) {
        self.snapshot.volume = (self.snapshot.volume + delta).clamp(0.0, 1.0);
        let _ = self.engine.set_volume(self.snapshot.volume);
    }

    fn position_from(&self, x: f32) -> f64 {
        let duration = self.engine.duration();
        if duration <= 0.0 {
            return 0.0;
        }
        Self::ratio_in(&self.layout.progress, x) * duration
    }

    /// 队列里点到了第几行。顶部留了一行标题的高度。
    fn queue_index_at(&self, y: f32) -> Option<i64> {
        let row = self.layout.row_height;
        let top = self.layout.queue.top + row;
        if row <= 0.0 || y < top {
            return None;
        }
        let index = ((y - top + self.queue_scroll) / row).floor() as i64;
        if index >= 0 && index < self.snapshot.tracks.len() as i64 {
            Some(index)
        } else {
            None
        }
    }

    unsafe fn on_left_down(&mut self, x: f32, y: f32) {
        let layout = self.layout;
        // 关闭和"回到完整模式"走同一条出口：这个窗口一旦没了，整个应用就没界面了，
        // 所以关窗只能是把完整界面拉回来，不是退进程。
        if contains(&layout.btn_close, x, y) {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        } else if contains(&layout.btn_full, x, y) {
            let _ = PostMessageW(Some(self.hwnd), WM_RETURN_TO_FULL, WPARAM(0), LPARAM(0));
        } else if contains(&layout.btn_min, x, y) {
            let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
        } else if contains(&layout.btn_play, x, y) {
            self.toggle_play();
        } else if contains(&layout.btn_prev, x, y) {
            self.step(-1, true);
        } else if contains(&layout.btn_next, x, y) {
            self.step(1, true);
        } else if contains(&layout.btn_loop, x, y) {
            self.snapshot.loop_mode = (self.snapshot.loop_mode + 1) % 3;
        } else if contains(&layout.volume_icon, x, y) {
            self.snapshot.muted = !self.snapshot.muted;
            let _ = self.engine.set_muted(self.snapshot.muted);
        } else if contains(&layout.volume_hit, x, y) {
            self.drag = Drag::Volume;
            SetCapture(self.hwnd);
            self.set_volume_from(x);
        } else if contains(&layout.progress_hit, x, y) {
            // 拖的过程只改本地预览值，松手才真的 seek，免得一路拖一路重新缓冲。
            self.drag = Drag::Progress;
            SetCapture(self.hwnd);
            self.scrub = Some(self.position_from(x));
        } else if contains(&layout.queue, x, y) {
            if let Some(index) = self.queue_index_at(y) {
                if index != self.snapshot.index {
                    self.snapshot.index = index;
                    self.snapshot.position = 0.0;
                    self.failures = 0;
                    self.begin_current(false);
                }
            }
        } else {
            return;
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn on_mouse_move(&mut self, x: f32) {
        match self.drag {
            Drag::Progress => self.scrub = Some(self.position_from(x)),
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
            if let Some(target) = self.scrub.take() {
                let _ = self.engine.seek(target);
                self.snapshot.position = target;
            }
        }
        let _ = ReleaseCapture();
        self.drag = Drag::None;
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    unsafe fn on_wheel(&mut self, delta: i16, x: f32, y: f32) {
        let notches = delta as f32 / 120.0;
        if contains(&self.layout.queue, x, y) {
            let row = self.layout.row_height;
            // 队列可视行数要扣掉顶上那行标题。
            let visible = (((self.layout.queue.bottom - self.layout.queue.top) / row) - 1.0).max(1.0);
            let max = (self.snapshot.tracks.len() as f32 - visible).max(0.0) * row;
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
        self.snapshot.position = target;
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
            self.snapshot.position = self.engine.position();
        }
        self.snapshot.saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|delta| delta.as_secs() as i64)
            .unwrap_or(0);
        let _ = self.engine.pause();
        let _ = snapshot::save(&self.snapshot);
        (self.on_return)(self.snapshot.clone());
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
            .snapshot
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
        Self::draw_icon(target, Icon::FullMode, &l.btn_full, &brushes.dim, icons);
        Self::draw_icon(target, Icon::Minimize, &l.btn_min, &brushes.dim, icons);
        Self::draw_icon(target, Icon::Close, &l.btn_close, &brushes.dim, icons);
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
                rect: l.cover,
                radiusX: radius,
                radiusY: radius,
            },
            &brushes.panel,
        );
        match self.cover.as_ref() {
            Some(bitmap) => {
                target.PushAxisAlignedClip(&l.cover, D2D1_ANTIALIAS_MODE_ALIASED);
                target.DrawBitmap(
                    bitmap,
                    Some(&l.cover),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                target.PopAxisAlignedClip();
            }
            None => Self::draw_icon(
                target,
                Icon::Note,
                &l.cover,
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
        let (title, artist, album) = match self.snapshot.current() {
            Some(track) => (
                track.title.as_str(),
                track.artist.as_str(),
                track.album.as_str(),
            ),
            None => ("没有正在播放的曲目", "", ""),
        };
        Self::text(target, title, &formats.title, &l.title, &brushes.text);

        // 副标题这一行是复用的：有话要说的时候（失败、缓冲）优先说话。
        if let Some(notice) = self.notice {
            Self::text(target, notice, &formats.sub, &l.subtitle, &brushes.accent);
            return;
        }
        if self.engine.is_busy() {
            Self::text(target, "缓冲中…", &formats.sub, &l.subtitle, &brushes.dim);
            return;
        }
        let subtitle = if album.is_empty() {
            artist.to_string()
        } else {
            format!("{artist} · {album}")
        };
        Self::text(target, &subtitle, &formats.sub, &l.subtitle, &brushes.dim);
    }

    unsafe fn draw_progress(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let duration = self.engine.duration();
        let position = self.display_position();
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
        target.FillRoundedRectangle(&rounded(bar), &brushes.line);
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
        Self::text(target, &clock, &formats.time, &l.time_row, &brushes.dim);
    }

    unsafe fn draw_controls(
        &self,
        target: &ID2D1HwndRenderTarget,
        brushes: &Brushes,
        formats: &Formats,
    ) {
        let l = &self.layout;
        let small = formats.icon_small.as_ref();
        let (loop_icon, loop_brush) = match self.snapshot.loop_mode {
            1 => (Icon::LoopOne, &brushes.accent),
            2 => (Icon::Shuffle, &brushes.accent),
            _ => (Icon::LoopSequence, &brushes.dim),
        };
        Self::draw_icon(target, loop_icon, &l.btn_loop, loop_brush, small);
        Self::draw_icon(
            target,
            Icon::Prev,
            &l.btn_prev,
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
            &play,
            &brushes.text,
            formats.icon_large.as_ref(),
        );
        Self::draw_icon(
            target,
            Icon::Next,
            &l.btn_next,
            &brushes.text,
            formats.icon_medium.as_ref(),
        );

        let silent = self.snapshot.muted || self.snapshot.volume <= 0.0;
        let volume_icon = if silent { Icon::Muted } else { Icon::Volume };
        Self::draw_icon(target, volume_icon, &l.volume_icon, &brushes.dim, small);
        let slider = l.volume_slider;
        let radius = (slider.bottom - slider.top) / 2.0;
        let rounded = |area: D2D_RECT_F| D2D1_ROUNDED_RECT {
            rect: area,
            radiusX: radius,
            radiusY: radius,
        };
        target.FillRoundedRectangle(&rounded(slider), &brushes.line);
        let level = if silent {
            0.0
        } else {
            self.snapshot.volume as f32
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
        if self.lyrics.is_empty() {
            Self::text(
                target,
                "没有歌词",
                &formats.lyric_dim,
                &l.lyrics,
                &brushes.line,
            );
            return;
        }
        target.PushAxisAlignedClip(&l.lyrics, D2D1_ANTIALIAS_MODE_ALIASED);
        let row = l.row_height;
        let center = (l.lyrics.top + l.lyrics.bottom) / 2.0;
        let total = self.lyrics.len() as i64;
        let translated = self
            .lyrics
            .get(self.active_line.max(0) as usize)
            .map(|line| line.translation.is_some())
            .unwrap_or(false);

        for offset in -2i64..=2 {
            let index = self.active_line + offset;
            if index < 0 || index >= total {
                continue;
            }
            // 当前行有译文时，译文占掉了下一行的位置。
            if offset == 1 && translated && self.active_line >= 0 {
                continue;
            }
            let line = &self.lyrics[index as usize];
            let top = center + offset as f32 * row - row / 2.0 + self.lyric_shift;
            let area = rect(l.lyrics.left, top, l.lyrics.right, top + row);
            if offset == 0 && self.active_line >= 0 {
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
                rect: l.queue,
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
        let count = self.snapshot.tracks.len();
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
        for (index, track) in self.snapshot.tracks.iter().enumerate() {
            let bottom = top + row;
            if bottom > body_top && top < l.queue.bottom {
                let current = index as i64 == self.snapshot.index;
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

fn fmt_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "00:00".to_string();
    }
    let total = seconds as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// 图标字体缺席时的替代画法。只用矩形和椭圆，不碰路径几何——
/// 这条分支在 Win10 之后基本不会走到，简陋一点比画不出来好。
#[derive(Clone, Copy)]
enum Fallback {
    TriangleRight,
    TriangleLeft,
    TwoBars,
    Bar,
    Cross,
    Frame,
    Dot,
}

impl Icon {
    fn fallback(self) -> Fallback {
        match self {
            Icon::Play | Icon::Next | Icon::Volume | Icon::Muted => Fallback::TriangleRight,
            Icon::Prev => Fallback::TriangleLeft,
            Icon::Pause => Fallback::TwoBars,
            Icon::Minimize => Fallback::Bar,
            Icon::Close => Fallback::Cross,
            Icon::FullMode | Icon::LoopSequence | Icon::LoopOne | Icon::Shuffle => Fallback::Frame,
            Icon::Note => Fallback::Dot,
        }
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
            if contains(&l.title_bar, x, y)
                && !contains(&l.btn_full, x, y)
                && !contains(&l.btn_min, x, y)
                && !contains(&l.btn_close, x, y)
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









