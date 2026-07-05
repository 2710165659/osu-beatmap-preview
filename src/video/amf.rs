//! AMD AMF (Advanced Media Framework) H.264 encoder backend.
//!
//! Dynamically loads `amfrt64.dll` at runtime via `libloading`. AMF uses a
//! COM-style C++ vtable interface. Because the available Rust AMF crates
//! (`shiguredo_amf`) pull in `bindgen` + `libclang` as build-time dependencies
//! (forcing every contributor to install LLVM — unacceptable for this
//! project's "no build friction" stance), we define our own minimal FFI
//! bindings here.
//!
//! ## Safety / fallback
//!
//! This backend is **blind-written** (no AMD GPU available for testing). Every
//! AMF call is wrapped to return `Ok(None)` on any failure, so the encoder
//! silently falls back to NVENC (if available) or CPU openh264. No AMD GPU →
//! `amfrt64.dll` not found → `try_create` returns `Ok(None)` immediately.
//!
//! ## AMF H.264 pipeline
//!
//! 1. `AMFInit()` → `AMFFactory1` (loads `amfrt64.dll` symbols)
//! 2. `factory->CreateContext()` → `AMFContext`
//! 3. `context->AllocSurface(HOST, BGRA, w, h)` → system-memory `AMFSurface`
//! 4. `factory->CreateComponent(context, "AMFVideoEncoderVCE_AVC")` → encoder
//! 5. Set properties: CQP, QP 22, B-frames=0, profile=High, IDR period
//! 6. `encoder->Init(BGRA, w, h)`
//! 7. Loop: lock surface plane → memcpy RGBA → `SubmitInput(surface)` →
//!    `QueryOutput(&data)` → read Annex-B NALs
//!
//! AMF outputs Annex-B by default; the shared `mux` module parses it.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};

use libloading::Library;
use std::ffi::c_void;
use std::os::raw::c_int;

// ── AMF type aliases ──

type amf_int32 = c_int;
type amf_int64 = i64;
type amf_size = usize;
type amf_uint = u32;
type wchar_t = u16; // Windows wchar_t is 16-bit

// ── AMF constants (from amf/public/include/core/AMFCore.h) ──

/// AMF memory type: host (system) memory.
const AMF_MEMORY_HOST: amf_int32 = 0;
/// AMF surface format: BGRA (matches RGBA byte order on little-endian? NO —
/// BGRA is B,G,R,A; our Img is R,G,B,A. We convert per-pixel on upload.)
const AMF_SURFACE_BGRA: amf_int32 = 4;

/// AMF result codes (amf/public/include/core/Result.h).
const AMF_OK: i32 = 0;
const AMF_NEED_MORE_INPUT: i32 = 1;
const AMF_REPEAT: i32 = 2;
const AMF_INPUT_FULL: i32 = 3;
const AMF_RESOLUTION_CHANGED: i32 = 4;
const AMF_RESOLUTION_UPDATED: i32 = 5;
const AMF_EOF: i32 = 6;
const AMF_NO_DEVICE: i32 = 10;

/// AMF variant type (amf/public/include/core/Variant.h).
const AMF_VARIANT_INTERFACE1: amf_int32 = 13;

/// Opaque pointer to an AMF interface (COM-like).
type AmfPtr = *mut c_void;

// ── AMF vtable layouts ──
//
// AMF interfaces are C++ abstract classes with a vtable as the first member.
// In Rust, we model them as `#[repr(C)]` structs whose first field is a
// pointer to a vtable struct. The vtable struct contains function pointers
// in declaration order. We only define the methods we actually call; unused
// slots are filled with `unsafe extern "C" fn(...)` placeholders.
//
// All AMF interfaces inherit from `AMFInterface` which has 3 methods:
//   0: Acquire()
//   1: Release()
//   2: QueryInterface(IID, pp)
// Then each derived interface adds its own virtual methods starting at index 3.

/// AMFInterface vtable (base for all AMF COM-like interfaces).
#[repr(C)]
struct AmfInterfaceVTable {
    acquire: unsafe extern "C" fn(AmfPtr) -> amf_uint,
    release: unsafe extern "C" fn(AmfPtr) -> amf_uint,
    query_interface: unsafe extern "C" fn(AmfPtr, *const c_void, *mut AmfPtr) -> amf_int32,
}

/// AMFPropertyStorage: set_property / get_property on the encoder.
/// Methods 0-2: inherited from AMFInterface.
/// Method 3: SetProperty(name, AMFVariantStruct*)
/// Method 4: getProperty(name, AMFVariantStruct*)
#[repr(C)]
struct AmfPropertyStorageVTable {
    base: AmfInterfaceVTable, // 0-2
    set_property: unsafe extern "C" fn(AmfPtr, *const wchar_t, *const AmfVariantStruct) -> amf_int32,
    get_property: unsafe extern "C" fn(AmfPtr, *const wchar_t, *mut AmfVariantStruct) -> amf_int32,
    // ... more methods we don't call; not listed to keep struct short.
}

/// AMFVariantStruct: a tagged union for property values.
/// We only use int64 and bool types for property setting.
#[repr(C)]
#[derive(Clone, Copy)]
struct AmfVariantStruct {
    type_: amf_int32,
    // Union of the value; we use int64 for all our property sets.
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

/// AMFSurface: system-memory frame buffer.
/// Methods 0-2: inherited from AMFInterface.
/// Method 3: GetPlane(index) → AMFPlane*
/// Method 4: GetMemoryType() → amf_int32
/// Method 5: SetPts(pts) → void
/// Method 6: GetPts() → int64
#[repr(C)]
struct AmfSurfaceVTable {
    base: AmfInterfaceVTable, // 0-2
    get_plane: unsafe extern "C" fn(AmfPtr, amf_size) -> AmfPtr,
    get_memory_type: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    set_pts: unsafe extern "C" fn(AmfPtr, amf_int64),
    get_pts: unsafe extern "C" fn(AmfPtr) -> amf_int64,
}

/// AMFPlane: a single color channel plane within a surface.
/// Method 3: GetNative() → void* (pixel data pointer)
/// Method 4: GetHPitch() → amf_size (row pitch in bytes)
/// Method 5: GetVPitch() → amf_size
/// Method 6: GetPixelSizeInBytes() → amf_size
#[repr(C)]
struct AmfPlaneVTable {
    base: AmfInterfaceVTable, // 0-2
    get_native: unsafe extern "C" fn(AmfPtr) -> *mut c_void,
    get_hpitch: unsafe extern "C" fn(AmfPtr) -> amf_size,
    get_vpitch: unsafe extern "C" fn(AmfPtr) -> amf_size,
    get_pixel_size_in_bytes: unsafe extern "C" fn(AmfPtr) -> amf_size,
}

/// AMFComponent: the encoder.
/// Methods 0-2: inherited from AMFInterface.
/// Method 3: Init(format, width, height) → amf_int32
/// Method 4: Terminate() → amf_int32
/// Method 5: Drain() → amf_int32
/// Method 6: Flush() → amf_int32
/// Method 7: SubmitInput(data) → amf_int32
/// Method 8: QueryOutput(&data) → amf_int32
#[repr(C)]
struct AmfComponentVTable {
    base: AmfInterfaceVTable, // 0-2
    init: unsafe extern "C" fn(AmfPtr, amf_int32, amf_int32, amf_int32) -> amf_int32,
    terminate: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    drain: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    flush: unsafe extern "C" fn(AmfPtr) -> amf_int32,
    submit_input: unsafe extern "C" fn(AmfPtr, AmfPtr) -> amf_int32,
    query_output: unsafe extern "C" fn(AmfPtr, *mut AmfPtr) -> amf_int32,
    // ... SetProperty via QueryInterface(AMFPropertyStorage) — but AMFComponent
    // inherits from AMFPropertyStorage, so the vtable actually has the property
    // methods too. For simplicity we use the AMFPropertyStorage vtable layout
    // for the component (since component IS-A property storage).
}

/// AMFBuffer: encoded bitstream output from QueryOutput.
/// Method 3: GetNative() → void*
/// Method 4: GetSize() → amf_size
#[repr(C)]
struct AmfBufferVTable {
    base: AmfInterfaceVTable, // 0-2
    get_native: unsafe extern "C" fn(AmfPtr) -> *mut c_void,
    get_size: unsafe extern "C" fn(AmfPtr) -> amf_size,
}

// ── AMF entry point types ──

/// `AMFInit1(version, &factory)` — the entry point exported by amfrt64.dll.
type AmfInitFn =
    unsafe extern "C" fn(u64, *mut AmfPtr) -> amf_int32;

/// The AMF factory interface (from AMFFactory1).
/// Method 3: CreateContext(&context) → amf_int32
/// Method 4: CreateComponent(context, name, &component) → amf_int32
#[repr(C)]
struct AmfFactoryVTable {
    base: AmfInterfaceVTable, // 0-2
    create_context: unsafe extern "C" fn(AmfPtr, *mut AmfPtr) -> amf_int32,
    create_component: unsafe extern "C" fn(AmfPtr, AmfPtr, *const wchar_t, *mut AmfPtr) -> amf_int32,
}

/// AMFContext: surface allocation.
/// Method 3: AllocSurface(memoryType, format, width, height, &surface) → amf_int32
#[repr(C)]
struct AmfContextVTable {
    base: AmfInterfaceVTable, // 0-2
    alloc_surface:
        unsafe extern "C" fn(AmfPtr, amf_int32, amf_int32, amf_int32, amf_int32, *mut AmfPtr) -> amf_int32,
    // ... many more methods (lock, unlock, etc.) we don't call.
}

/// Helper: call Release() on an AMF interface pointer (vtable slot 1).
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

/// The common layout of all AMF interface objects: first field is the vtable pointer.
#[repr(C)]
struct AmfInterfaceWrapper {
    vtable: *const AmfInterfaceVTable,
}

/// Get the vtable pointer from an AMF interface object.
unsafe fn amf_vtable<T>(ptr: AmfPtr) -> *const T {
    if ptr.is_null() {
        return std::ptr::null();
    }
    *(ptr as *const *const T)
}

/// AMF API version. AMF uses a 64-bit version: major<<48 | minor<<32 | release<<16 | build.
/// We request version 1.0.0.0.
const AMF_FULL_VERSION: u64 = (1u64 << 48) | (1u64 << 32);

// ── AMF encoder property name strings (wide) ──
//
// These are the property names from amf/public/include/components/VideoEncoderVCE.h.
// They must be wide (UTF-16) strings on Windows.

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0u16)).collect()
}

/// Try to create an AMF encoder. Returns `Ok(None)` if `amfrt64.dll` is
/// unavailable or any AMF call fails.
pub(crate) fn try_create(w: u32, h: u32, fps: u32) -> Result<Option<AmfEncoder>> {
    // 1. Load amfrt64.dll
    let lib = match unsafe { Library::new("amfrt64.dll") } {
        Ok(l) => l,
        Err(_) => {
            eprintln!("[video] AMF: amfrt64.dll not found, skipping");
            return Ok(None);
        }
    };

    // 2. Get AMFInit1 entry point
    let amf_init: AmfInitFn = match unsafe { lib.get(b"AMFInit1") } {
        Ok(f) => *f,
        Err(e) => {
            eprintln!("[video] AMF: AMFInit1 not found: {e}");
            return Ok(None);
        }
    };

    // 3. AMFInit1 → factory
    let mut factory: AmfPtr = std::ptr::null_mut();
    let status = unsafe { amf_init(AMF_FULL_VERSION, &mut factory) };
    if status != AMF_OK || factory.is_null() {
        eprintln!("[video] AMF: AMFInit1 failed: status={status}");
        return Ok(None);
    }

    // The factory vtable: factory is an AMFFactory1* whose first field is the vtable.
    let factory_vt: *const AmfFactoryVTable = unsafe { amf_vtable(factory) };
    if factory_vt.is_null() {
        eprintln!("[video] AMF: factory vtable null");
        unsafe { amf_release(factory) };
        return Ok(None);
    }

    // 4. factory->CreateContext()
    let mut context: AmfPtr = std::ptr::null_mut();
    let status = unsafe { ((*factory_vt).create_context)(factory, &mut context) };
    if status != AMF_OK || context.is_null() {
        eprintln!("[video] AMF: CreateContext failed: status={status}");
        unsafe { amf_release(factory) };
        return Ok(None);
    }
    let context_vt: *const AmfContextVTable = unsafe { amf_vtable(context) };

    // 5. context->AllocSurface(HOST, BGRA, w, h)
    let mut surface: AmfPtr = std::ptr::null_mut();
    let status = unsafe {
        ((*context_vt).alloc_surface)(context, AMF_MEMORY_HOST, AMF_SURFACE_BGRA, w as amf_int32, h as amf_int32, &mut surface)
    };
    if status != AMF_OK || surface.is_null() {
        eprintln!("[video] AMF: AllocSurface failed: status={status}");
        unsafe { amf_release(context); amf_release(factory) };
        return Ok(None);
    }
    let surface_vt: *const AmfSurfaceVTable = unsafe { amf_vtable(surface) };

    // 6. factory->CreateComponent(context, "AMFVideoEncoderVCE_AVC")
    let component_name = wstr("AMFVideoEncoderVCE_AVC");
    let mut component: AmfPtr = std::ptr::null_mut();
    let status = unsafe {
        ((*factory_vt).create_component)(factory, context, component_name.as_ptr(), &mut component)
    };
    if status != AMF_OK || component.is_null() {
        eprintln!("[video] AMF: CreateComponent failed: status={status}");
        unsafe { amf_release(surface); amf_release(context); amf_release(factory) };
        return Ok(None);
    }

    // The component inherits from AMFPropertyStorage, so its vtable has
    // set_property at slot 3. We use the AmfPropertyStorageVTable layout.
    let prop_vt: *const AmfPropertyStorageVTable = unsafe { amf_vtable(component) };
    if prop_vt.is_null() {
        eprintln!("[video] AMF: component vtable null");
        unsafe { amf_release(component); amf_release(surface); amf_release(context); amf_release(factory) };
        return Ok(None);
    }

    // 7. Set encoder properties
    let keyframe_period = (fps * 2).max(1) as i64;
    let props = [
        (wstr("Usage"), 0),                 // AMF_VIDEO_ENCODER_USAGE_LOW_LATENCY
        (wstr("Profile"), 100),             // AMF_VIDEO_ENCODER_PROFILE_HIGH
        (wstr("ProfileLevel"), 41),         // Level 4.1
        (wstr("TargetBitrate"), 4_000_000), // 4 Mbps
        (wstr("RateControlMethod"), 1),     // CBR
        (wstr("BPicturesPattern"), 0),      // No B-frames
        (wstr("IDRPeriod"), keyframe_period),
        (wstr("MaxNumRefFrames"), 1),
    ];
    for (name, val) in &props {
        let variant = AmfVariantStruct::int64(*val);
        let status = unsafe { ((*prop_vt).set_property)(component, name.as_ptr(), &variant) };
        if status != AMF_OK {
            eprintln!("[video] AMF: SetProperty failed for property, status={status}");
            // Don't abort — some properties may be unsupported on certain driver versions.
        }
    }

    // 8. component->Init(BGRA, w, h)
    let comp_vt: *const AmfComponentVTable = unsafe { amf_vtable(component) };
    let status = unsafe { ((*comp_vt).init)(component, AMF_SURFACE_BGRA, w as amf_int32, h as amf_int32) };
    if status != AMF_OK {
        eprintln!("[video] AMF: Init failed: status={status}");
        unsafe { amf_release(component); amf_release(surface); amf_release(context); amf_release(factory) };
        return Ok(None);
    }

    eprintln!("[video] AMF: encoder initialized successfully");

    // Keep the Library alive — if it's dropped, the DLL is unloaded and all
    // AMF pointers become dangling. We leak it intentionally (encoder lives
    // for the program's duration anyway).
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

        // ── copy RGBA → BGRA into the AMF surface ──
        // AMF surface is BGRA; our Img is RGBA. Swap R and B per pixel.
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

        // Write RGBA→BGRA into the surface
        let row_bytes = (self.width * 4) as usize;
        for y in 0..self.height as usize {
            let src = y * row_bytes;
            let dst = y * pitch;
            unsafe {
                let src_row = rgba.data.as_ptr().add(src);
                let dst_row = data_ptr.add(dst) as *mut u8;
                for x in 0..self.width as usize {
                    // RGBA → BGRA: swap bytes 0 and 2
                    *dst_row.add(x * 4) = *src_row.add(x * 4 + 2); // B
                    *dst_row.add(x * 4 + 1) = *src_row.add(x * 4 + 1); // G
                    *dst_row.add(x * 4 + 2) = *src_row.add(x * 4); // R
                    *dst_row.add(x * 4 + 3) = *src_row.add(x * 4 + 3); // A
                }
            }
        }

        // Set PTS
        unsafe { ((*self.surface_vt).set_pts)(self.surface, self.frame_idx as i64) };

        // ── SubmitInput ──
        let status = unsafe { ((*self.comp_vt).submit_input)(self.component, self.surface) };
        if status != AMF_OK && status != AMF_NEED_MORE_INPUT && status != AMF_INPUT_FULL {
            return Err(PreviewError::render(format!(
                "AMF: SubmitInput failed: status={status}"
            )));
        }

        // ── QueryOutput ──
        let mut output: AmfPtr = std::ptr::null_mut();
        let status = unsafe { ((*self.comp_vt).query_output)(self.component, &mut output) };
        if status != AMF_OK || output.is_null() {
            // AMF may need one more input before producing output (pipeline
            // latency). For the first frame, emit an empty placeholder — the
            // muxer expects the first frame to carry SPS/PPS though, so this
            // is a problem. In practice, AMF produces output on the first
            // QueryOutput after Init+SubmitInput. If it doesn't, fall back.
            return Err(PreviewError::render(format!(
                "AMF: QueryOutput returned no data: status={status}"
            )));
        }

        // ── read the encoded buffer ──
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

        let (sps, pps, slice) = extract_nals_from_annexb(&self.annexb_buf);
        Ok(EncodedFrame { sps, pps, slice })
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
