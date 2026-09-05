//! AMD AMF（Advanced Media Framework）H.264 编码后端。
//!
//! 通过 `libloading` 在运行时动态加载 `amfrt64.dll`。AMF 使用类似 COM 的 C++ vtable 接口。
//! 现有 Rust AMF crate（`shiguredo_amf`）会引入 `bindgen` + `libclang` 构建依赖，
//! 要求所有贡献者安装 LLVM，不符合项目降低构建门槛的目标，因此这里定义最小 FFI 绑定。
//!
//! ## 安全性与回退
//!
//! 此后端是在没有 AMD GPU 的环境下编写的，未经过实际硬件测试。所有 AMF 调用都包装为
//! 失败时返回 `Ok(None)`，编码器会静默回退到 NVENC（若可用）或 CPU openh264。
//! 没有 AMD GPU 或找不到 `amfrt64.dll` 时，`try_create` 会立即返回 `Ok(None)`。
//!
//! ## AMF H.264 流程
//!
//! 1. `AMFInit()` → `AMFFactory1`（加载 `amfrt64.dll` 符号）。
//! 2. `factory->CreateContext()` → `AMFContext`。
//! 3. `context->AllocSurface(HOST, BGRA, w, h)` → 系统内存 `AMFSurface`。
//! 4. `factory->CreateComponent(context, "AMFVideoEncoderVCE_AVC")` → 编码器。
//! 5. 设置属性：速度预设、900 kbps CBR、无 B 帧、High profile。
//! 6. `encoder->Init(BGRA, w, h)`。
//! 7. 循环：锁定 surface 平面 → memcpy RGBA → `SubmitInput(surface)` →
//!    `QueryOutput(&data)` → 读取 Annex-B NAL。
//!
//! AMF 默认输出 Annex-B，由共享 `mux` 模块解析。

use crate::domain::errors::{PreviewError, Result};
use crate::render::canvas::Img;

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};

use libloading::Library;
use std::ffi::c_void;
use std::os::raw::c_int;

// ── AMF 类型别名 ──

type amf_int32 = c_int;
type amf_int64 = i64;
type amf_size = usize;
type amf_uint = u32;
type wchar_t = u16; // Windows wchar_t is 16-bit

// ── AMF 常量（来自 amf/public/include/core/AMFCore.h） ──

/// AMF 内存类型：主机（系统）内存。
const AMF_MEMORY_HOST: amf_int32 = 0;
/// AMF surface 格式：BGRA（与小端 RGBA 字节序不同；Img 使用 R,G,B,A，上传时逐像素转换）。
const AMF_SURFACE_BGRA: amf_int32 = 4;

/// AMF 结果码（amf/public/include/core/Result.h）。
const AMF_OK: i32 = 0;
const AMF_NEED_MORE_INPUT: i32 = 1;
const AMF_REPEAT: i32 = 2;
const AMF_INPUT_FULL: i32 = 3;
const AMF_RESOLUTION_CHANGED: i32 = 4;
const AMF_RESOLUTION_UPDATED: i32 = 5;
const AMF_EOF: i32 = 6;
const AMF_NO_DEVICE: i32 = 10;

/// AMF 变体类型（amf/public/include/core/Variant.h）。
const AMF_VARIANT_INTERFACE1: amf_int32 = 13;

/// AMF 接口（类 COM）的不透明指针。
type AmfPtr = *mut c_void;

// ── AMF vtable 布局 ──
//
// AMF 接口是 C++ 抽象类，第一个成员是 vtable。Rust 中用 `#[repr(C)]` 结构体表示，
// 首字段指向 vtable 结构体；vtable 按声明顺序保存函数指针。这里只定义实际调用的方法，
// 未使用的槽位用 `unsafe extern "C" fn(...)` 占位。
//
// 所有 AMF 接口继承包含 3 个方法的 `AMFInterface`：
//   0：Acquire()；1：Release()；2：QueryInterface(IID, pp)。
// 派生接口从索引 3 开始追加自己的虚方法。

/// AMFInterface vtable（所有类 COM AMF 接口的基类）。
#[repr(C)]
struct AmfInterfaceVTable {
    acquire: unsafe extern "C" fn(AmfPtr) -> amf_uint,
    release: unsafe extern "C" fn(AmfPtr) -> amf_uint,
    query_interface: unsafe extern "C" fn(AmfPtr, *const c_void, *mut AmfPtr) -> amf_int32,
}

/// AMFPropertyStorage：在编码器上执行 set_property / get_property。
/// 方法 0-2 继承自 AMFInterface；方法 3 为 SetProperty，方法 4 为 getProperty。
#[repr(C)]
struct AmfPropertyStorageVTable {
    base: AmfInterfaceVTable, // 0-2
    set_property:
        unsafe extern "C" fn(AmfPtr, *const wchar_t, *const AmfVariantStruct) -> amf_int32,
    get_property: unsafe extern "C" fn(AmfPtr, *const wchar_t, *mut AmfVariantStruct) -> amf_int32,
    // 还有其它未调用的方法，为保持结构简短不在此列出。
}

/// AMFVariantStruct：带标签的属性值联合体。属性设置仅使用 int64 和 bool 类型。
#[repr(C)]
#[derive(Clone, Copy)]
struct AmfVariantStruct {
    type_: amf_int32,
    // 值联合体；当前所有属性设置都使用 int64。
    value_int64: amf_int64,
}

impl AmfVariantStruct {
    fn int64(v: i64) -> Self {
        Self {
            type_: 5, // AMF_VARIANT_INT64
            value_int64: v,
        }
    }
}

/// AMFSurface：系统内存帧缓冲区。方法 0-2 继承自 AMFInterface；
/// 方法 3-6 分别获取平面、内存类型以及设置/读取 PTS。
#[repr(C)]
struct AmfSurfaceVTable {
    base: AmfInterfaceVTable, // 0-2
    get_plane: unsafe extern "C" fn(AmfPtr, amf_size) -> AmfPtr,
    get_memory_type: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    set_pts: unsafe extern "C" fn(AmfPtr, amf_int64),
    get_pts: unsafe extern "C" fn(AmfPtr) -> amf_int64,
}

/// AMFPlane：surface 中的单个颜色通道平面。方法 3-6 获取像素指针、行跨度、
/// 垂直跨度和单像素字节数。
#[repr(C)]
struct AmfPlaneVTable {
    base: AmfInterfaceVTable, // 0-2
    get_native: unsafe extern "C" fn(AmfPtr) -> *mut c_void,
    get_hpitch: unsafe extern "C" fn(AmfPtr) -> amf_size,
    get_vpitch: unsafe extern "C" fn(AmfPtr) -> amf_size,
    get_pixel_size_in_bytes: unsafe extern "C" fn(AmfPtr) -> amf_size,
}

/// AMFComponent：编码器。方法 0-2 继承自 AMFInterface；方法 3-8 依次为初始化、
/// 终止、排空、刷新、提交输入和查询输出。
#[repr(C)]
struct AmfComponentVTable {
    base: AmfInterfaceVTable, // 0-2
    init: unsafe extern "C" fn(AmfPtr, amf_int32, amf_int32, amf_int32) -> amf_int32,
    terminate: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    drain: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    flush: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    submit_input: unsafe extern "C" fn(AmfPtr, AmfPtr) -> amf_int32,
    query_output: unsafe extern "C" fn(AmfPtr, *mut AmfPtr) -> amf_int32,
    // SetProperty 通常通过 QueryInterface(AMFPropertyStorage) 调用；AMFComponent
    // 继承自 AMFPropertyStorage，因此其 vtable 也包含属性方法。为简化实现，
    // 组件直接使用 AMFPropertyStorage 的 vtable 布局。
}

/// AMFBuffer：QueryOutput 返回的编码字节流。方法 3 获取指针，方法 4 获取大小。
#[repr(C)]
struct AmfBufferVTable {
    base: AmfInterfaceVTable, // 0-2
    get_native: unsafe extern "C" fn(AmfPtr) -> *mut c_void,
    get_size: unsafe extern "C" fn(AmfPtr) -> amf_size,
}

// ── AMF 入口类型 ──

/// `AMFInit1(version, &factory)`：amfrt64.dll 导出的入口点。
type AmfInitFn = unsafe extern "C" fn(u64, *mut AmfPtr) -> amf_int32;

/// AMF 工厂接口（来自 AMFFactory1）。方法 3 创建上下文，方法 4 创建编码组件。
#[repr(C)]
struct AmfFactoryVTable {
    base: AmfInterfaceVTable, // 0-2
    create_context: unsafe extern "C" fn(AmfPtr, *mut AmfPtr) -> amf_int32,
    create_component:
        unsafe extern "C" fn(AmfPtr, AmfPtr, *const wchar_t, *mut AmfPtr) -> amf_int32,
}

/// AMFContext：负责分配 surface。方法 3 根据内存类型、格式和尺寸分配 surface。
#[repr(C)]
struct AmfContextVTable {
    base: AmfInterfaceVTable, // 0-2
    alloc_surface: unsafe extern "C" fn(
        AmfPtr,
        amf_int32,
        amf_int32,
        amf_int32,
        amf_int32,
        *mut AmfPtr,
    ) -> amf_int32,
    // 还有 lock、unlock 等许多未调用的方法。
}

/// 辅助函数：调用 AMF 接口指针的 Release()（vtable 槽位 1）。
unsafe fn amf_release(ptr: AmfPtr) {
    if ptr.is_null() {
        return;
    }
    let wrapper = &*(ptr as *const AmfInterfaceWrapper);
    if !wrapper.vtable.is_null() {
        let vt = &*wrapper.vtable;
        (vt.release)(ptr);
    }
}

/// 所有 AMF 接口对象的通用布局：第一个字段是 vtable 指针。
#[repr(C)]
struct AmfInterfaceWrapper {
    vtable: *const AmfInterfaceVTable,
}

/// 获取 AMF 接口对象的 vtable 指针。
unsafe fn amf_vtable<T>(ptr: AmfPtr) -> *const T {
    if ptr.is_null() {
        return std::ptr::null();
    }
    *(ptr as *const *const T)
}

/// AMF API 版本。AMF 使用 64 位版本号：major<<48 | minor<<32 | release<<16 | build。
/// 当前请求版本 1.0.0.0。
const AMF_FULL_VERSION: u64 = (1u64 << 48) | (1u64 << 32);

// ── AMF 编码器属性名称（宽字符串） ──
//
// 属性名称来自 amf/public/include/components/VideoEncoderVCE.h，
// Windows 上必须使用宽字符串（UTF-16）。

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0u16)).collect()
}

/// 尝试创建 AMF 编码器。`amfrt64.dll` 不可用或任一 AMF 调用失败时返回 `Ok(None)`。
pub(crate) fn try_create(w: u32, h: u32, fps: u32) -> Result<Option<AmfEncoder>> {
    // 1. 加载 amfrt64.dll。
    let lib = match unsafe { Library::new("amfrt64.dll") } {
        Ok(l) => l,
        Err(_) => {
            eprintln!("[video] AMF: amfrt64.dll not found, skipping");
            return Ok(None);
        }
    };

    // 2. 获取 AMFInit1 入口点。
    let amf_init: AmfInitFn = match unsafe { lib.get(b"AMFInit1") } {
        Ok(f) => *f,
        Err(e) => {
            eprintln!("[video] AMF: AMFInit1 not found: {e}");
            return Ok(None);
        }
    };

    // 3. 调用 AMFInit1 获取工厂。
    let mut factory: AmfPtr = std::ptr::null_mut();
    let status = unsafe { amf_init(AMF_FULL_VERSION, &mut factory) };
    if status != AMF_OK || factory.is_null() {
        eprintln!("[video] AMF: AMFInit1 failed: status={status}");
        return Ok(None);
    }

    // 工厂 vtable：factory 是 AMFFactory1*，其第一个字段为 vtable。
    let factory_vt: *const AmfFactoryVTable = unsafe { amf_vtable(factory) };
    if factory_vt.is_null() {
        eprintln!("[video] AMF: factory vtable null");
        unsafe { amf_release(factory) };
        return Ok(None);
    }

    // 4. 调用 factory->CreateContext()。
    let mut context: AmfPtr = std::ptr::null_mut();
    let status = unsafe { ((*factory_vt).create_context)(factory, &mut context) };
    if status != AMF_OK || context.is_null() {
        eprintln!("[video] AMF: CreateContext failed: status={status}");
        unsafe { amf_release(factory) };
        return Ok(None);
    }
    let context_vt: *const AmfContextVTable = unsafe { amf_vtable(context) };

    // 5. 调用 context->AllocSurface(HOST, BGRA, w, h)。
    let mut surface: AmfPtr = std::ptr::null_mut();
    let status = unsafe {
        ((*context_vt).alloc_surface)(
            context,
            AMF_MEMORY_HOST,
            AMF_SURFACE_BGRA,
            w as amf_int32,
            h as amf_int32,
            &mut surface,
        )
    };
    if status != AMF_OK || surface.is_null() {
        eprintln!("[video] AMF: AllocSurface failed: status={status}");
        unsafe {
            amf_release(context);
            amf_release(factory)
        };
        return Ok(None);
    }
    let surface_vt: *const AmfSurfaceVTable = unsafe { amf_vtable(surface) };

    // 6. 调用 factory->CreateComponent(context, "AMFVideoEncoderVCE_AVC")。
    let component_name = wstr("AMFVideoEncoderVCE_AVC");
    let mut component: AmfPtr = std::ptr::null_mut();
    let status = unsafe {
        ((*factory_vt).create_component)(factory, context, component_name.as_ptr(), &mut component)
    };
    if status != AMF_OK || component.is_null() {
        eprintln!("[video] AMF: CreateComponent failed: status={status}");
        unsafe {
            amf_release(surface);
            amf_release(context);
            amf_release(factory)
        };
        return Ok(None);
    }

    // 组件继承自 AMFPropertyStorage，因此 vtable 的槽位 3 是 set_property。
    // 此处使用 AmfPropertyStorageVTable 布局。
    let prop_vt: *const AmfPropertyStorageVTable = unsafe { amf_vtable(component) };
    if prop_vt.is_null() {
        eprintln!("[video] AMF: component vtable null");
        unsafe {
            amf_release(component);
            amf_release(surface);
            amf_release(context);
            amf_release(factory)
        };
        return Ok(None);
    }

    // 7. 设置编码器属性。
    let keyframe_period = (fps * 2).max(1) as i64;
    let props = [
        (wstr("Usage"), 0),         // AMF_VIDEO_ENCODER_USAGE_LOW_LATENCY
        (wstr("QualityPreset"), 1), // AMF_VIDEO_ENCODER_QUALITY_PRESET_SPEED
        (wstr("Profile"), 100),     // AMF_VIDEO_ENCODER_PROFILE_HIGH
        (wstr("ProfileLevel"), 41), // Level 4.1
        (
            wstr("TargetBitrate"),
            crate::infrastructure::config::current()
                .advance
                .video
                .VIDEO_BITRATE as i64,
        ),
        (wstr("RateControlMethod"), 1), // CBR
        (wstr("BPicturesPattern"), 0),  // No B-frames
        (wstr("IDRPeriod"), keyframe_period),
        (wstr("MaxNumRefFrames"), 1),
    ];
    for (name, val) in &props {
        let variant = AmfVariantStruct::int64(*val);
        let status = unsafe { ((*prop_vt).set_property)(component, name.as_ptr(), &variant) };
        if status != AMF_OK {
            eprintln!("[video] AMF: SetProperty failed for property, status={status}");
            // 不要中止：某些驱动版本可能不支持部分属性。
        }
    }

    // 8. 调用 component->Init(BGRA, w, h)。
    let comp_vt: *const AmfComponentVTable = unsafe { amf_vtable(component) };
    let status =
        unsafe { ((*comp_vt).init)(component, AMF_SURFACE_BGRA, w as amf_int32, h as amf_int32) };
    if status != AMF_OK {
        eprintln!("[video] AMF: Init failed: status={status}");
        unsafe {
            amf_release(component);
            amf_release(surface);
            amf_release(context);
            amf_release(factory)
        };
        return Ok(None);
    }

    eprintln!("[video] AMF: encoder initialized successfully");

    // 保持 Library 存活；如果被释放，DLL 会卸载，所有 AMF 指针都会悬空。
    // 这里有意泄漏该对象，因为编码器会存在于整个程序生命周期。
    std::mem::forget(lib);

    Ok(Some(AmfEncoder {
        factory,
        context,
        surface,
        surface_vt,
        component,
        comp_vt,
        prop_vt,
        width: w,
        height: h,
        frame_idx: u32::MAX, // will be set to 0 on first encode
        keyframe_period,
        annexb_buf: Vec::new(),
    }))
}

pub(crate) struct AmfEncoder {
    factory: AmfPtr,
    context: AmfPtr,
    surface: AmfPtr,
    surface_vt: *const AmfSurfaceVTable,
    component: AmfPtr,
    comp_vt: *const AmfComponentVTable,
    prop_vt: *const AmfPropertyStorageVTable,
    width: u32,
    height: u32,
    frame_idx: u32,
    keyframe_period: i64,
    annexb_buf: Vec<u8>,
}

impl FrameEncoder for AmfEncoder {
    fn encode(&mut self, rgba: &Img) -> Result<EncodedFrame> {
        if self.frame_idx == u32::MAX {
            self.frame_idx = 0;
        }

        // ── 将 RGBA 复制为 BGRA 到 AMF surface ──
        // AMF surface 使用 BGRA，而 Img 使用 RGBA，因此逐像素交换 R 与 B。
        let plane_ptr = unsafe { ((*self.surface_vt).get_plane)(self.surface, 0) };
        if plane_ptr.is_null() {
            return Err(PreviewError::render("AMF: surface plane null"));
        }
        let plane_vt: *const AmfPlaneVTable = unsafe { amf_vtable(plane_ptr) };
        if plane_vt.is_null() {
            return Err(PreviewError::render("AMF: plane vtable null"));
        }
        let data_ptr = unsafe { ((*plane_vt).get_native)(plane_ptr) };
        let pitch = unsafe { ((*plane_vt).get_hpitch)(plane_ptr) };
        if data_ptr.is_null() || pitch == 0 {
            return Err(PreviewError::render("AMF: plane data/pitch invalid"));
        }

        // 将 RGBA→BGRA 写入 surface。
        let row_bytes = (self.width * 4) as usize;
        for y in 0..self.height as usize {
            let src = y * row_bytes;
            let dst = y * pitch;
            unsafe {
                let src_row = rgba.data.as_ptr().add(src);
                let dst_row = data_ptr.add(dst) as *mut u8;
                for x in 0..self.width as usize {
                    // RGBA → BGRA：交换第 0 和第 2 字节。
                    *dst_row.add(x * 4) = *src_row.add(x * 4 + 2); // B
                    *dst_row.add(x * 4 + 1) = *src_row.add(x * 4 + 1); // G
                    *dst_row.add(x * 4 + 2) = *src_row.add(x * 4); // R
                    *dst_row.add(x * 4 + 3) = *src_row.add(x * 4 + 3); // A
                }
            }
        }

        // 设置 PTS。
        unsafe { ((*self.surface_vt).set_pts)(self.surface, self.frame_idx as i64) };

        // ── 提交输入 ──
        let status = unsafe { ((*self.comp_vt).submit_input)(self.component, self.surface) };
        if status != AMF_OK && status != AMF_NEED_MORE_INPUT && status != AMF_INPUT_FULL {
            return Err(PreviewError::render(format!(
                "AMF: SubmitInput failed: status={status}"
            )));
        }

        // ── 查询输出 ──
        let mut output: AmfPtr = std::ptr::null_mut();
        let status = unsafe { ((*self.comp_vt).query_output)(self.component, &mut output) };
        if status != AMF_OK || output.is_null() {
            // AMF 可能需要额外输入后才产生输出（流水线延迟）。首帧若输出空占位，
            // 会与封装器要求首帧携带 SPS/PPS 的规则冲突。实践中 AMF 会在 Init + SubmitInput
            // 后第一次 QueryOutput 产生输出；否则回退到其它编码器。
            return Err(PreviewError::render(format!(
                "AMF: QueryOutput returned no data: status={status}"
            )));
        }

        // ── 读取编码缓冲区 ──
        let buf_vt: *const AmfBufferVTable = unsafe { amf_vtable(output) };
        if buf_vt.is_null() {
            unsafe { amf_release(output) };
            return Err(PreviewError::render("AMF: buffer vtable null"));
        }
        let buf_ptr = unsafe { ((*buf_vt).get_native)(output) };
        let buf_size = unsafe { ((*buf_vt).get_size)(output) };
        unsafe { amf_release(output) };

        if buf_ptr.is_null() || buf_size == 0 {
            return Err(PreviewError::render("AMF: empty encoded buffer"));
        }

        self.annexb_buf.clear();
        unsafe {
            self.annexb_buf
                .extend_from_slice(std::slice::from_raw_parts(buf_ptr as *const u8, buf_size));
        }

        self.frame_idx += 1;

        let (sps, pps, slice, is_keyframe) = extract_nals_from_annexb(&self.annexb_buf);
        Ok(EncodedFrame {
            sps,
            pps,
            slice,
            is_keyframe,
        })
    }

    fn name(&self) -> &'static str {
        "AMF"
    }
}

impl Drop for AmfEncoder {
    fn drop(&mut self) {
        unsafe {
            if !self.component.is_null() {
                let vt: *const AmfComponentVTable = amf_vtable(self.component);
                if !vt.is_null() {
                    ((*vt).terminate)(self.component);
                }
                amf_release(self.component);
            }
            amf_release(self.surface);
            amf_release(self.context);
            amf_release(self.factory);
        }
    }
}
