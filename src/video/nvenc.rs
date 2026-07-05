//! NVIDIA NVENC H.264 encoder backend.
//!
//! Dynamically loads `nvEncodeAPI64.dll` at runtime via the `nvenc` crate
//! (which uses `libloading`). If the DLL is missing (no NVIDIA GPU / driver),
//! `try_create` returns `Ok(None)` and the caller falls through to the next
//! backend.
//!
//! ## Input path
//!
//! Uses a **D3D11 staging texture** (CPU-writable) as the NVENC input source.
//! Each frame: Map the texture → memcpy RGBA → Unmap → `register_resource_dx11`
//! → `encode_picture` → drop (auto unmap+unregister). The texture is created
//! once and reused across frames.
//!
//! This path is used instead of `create_input_buffer` + `InputBuffer::lock()`
//! because the crate's `InputBufferLock::drop` has a bug: it passes
//! `buffer_data_ptr` instead of the buffer handle to `unlock_input_buffer`.
//! The `RegisteredResource` wrapper (from `register_resource_dx11`) has a
//! correct `Drop` impl, so it's safe to use.
//!
//! ## Configuration
//!
//! - H.264, preset P3, LowLatency tuning (CQP + QP ~22, 0 B-frames by default)
//! - GOP = 2s of frames; NVENC emits SPS/PPS before the first IDR by default
//! - Output is Annex-B, parsed by the shared `mux` module

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};

use nvenc::bitstream::BitStream;
use nvenc::session::{InitParams, Session};
use nvenc::sys::enums::{
    NVencBufferFormat, NVencPicStruct, NVencPicType, NVencTuningInfo,
};
use nvenc::sys::guids::{NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P3_GUID};

/// Try to create an NVENC encoder. Returns `Ok(None)` if the NVENC DLL is
/// unavailable or session creation fails (e.g. no NVIDIA GPU).
pub(crate) fn try_create(w: u32, h: u32, fps: u32) -> Result<Option<NvencEncoder>> {
    match NvencEncoder::new(w, h, fps) {
        Ok(enc) => Ok(Some(enc)),
        Err(NvencInitError::Unavailable) => {
            eprintln!("[video] NVENC unavailable, falling back");
            Ok(None)
        }
        Err(NvencInitError::Failed(e)) => {
            eprintln!("[video] NVENC init failed: {e}, falling back");
            Ok(None)
        }
    }
}

enum NvencInitError {
    Unavailable,
    Failed(PreviewError),
}

pub(crate) struct NvencEncoder {
    encoder: nvenc::encoder::Encoder,
    bitstream: BitStream,
    /// Reused Annex-B assembly buffer.
    annexb_buf: Vec<u8>,
    frame_idx: u32,
    keyframe_period: u32,
    /// D3D11 device + staging texture (CPU-writable, registered with NVENC
    /// once at init and reused across all frames).
    d3d: D3D11Resources,
    /// Registered NVENC input resource — created once, reused every frame.
    /// Avoids the ~5ms/frame overhead of register+unmap+unregister.
    registered: nvenc::encoder::RegisteredResource,
    _device_guard: D3D11DeviceGuard,
}

unsafe impl Send for NvencEncoder {}

impl NvencEncoder {
    fn new(w: u32, h: u32, fps: u32) -> std::result::Result<Self, NvencInitError> {
        if w % 2 != 0 || h % 2 != 0 {
            return Err(NvencInitError::Failed(PreviewError::render(format!(
                "NVENC requires even dimensions, got {w}x{h}"
            ))));
        }

        // ── 1. create D3D11 device (NVENC session + staging texture) ──
        let device_guard = D3D11DeviceGuard::create().map_err(NvencInitError::Failed)?;

        // ── 2. open NVENC session ──
        let session = match Session::open_dx(&device_guard.device) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[video] NVENC open_dx failed: {e:?}");
                return Err(NvencInitError::Unavailable);
            }
        };

        // ── 3. get preset config ──
        let (session, mut config) = session
            .get_encode_preset_config_ex(
                NV_ENC_CODEC_H264_GUID,
                NV_ENC_PRESET_P3_GUID,
                NVencTuningInfo::LowLatency,
            )
            .map_err(|e| {
                NvencInitError::Failed(PreviewError::render(format!(
                    "NVENC get_preset_config failed: {e:?}"
                )))
            })?;

        // ── 4. override GOP length, enforce no B-frames ──
        let keyframe_period = (fps * 2).max(1);
        config.preset_cfg.gop_len = keyframe_period;
        config.preset_cfg.frame_interval_p = 1;

        // ── 5. init encoder ──
        let init_params = InitParams {
            encode_guid: NV_ENC_CODEC_H264_GUID,
            preset_guid: NV_ENC_PRESET_P3_GUID,
            aspect_ratio: [w, h],
            encode_config: &mut config.preset_cfg,
            tuning_info: NVencTuningInfo::LowLatency,
            buffer_format: NVencBufferFormat::ARGB,
            frame_rate: [fps, 1],
            resolution: [w, h],
            enable_ptd: true,
            max_encoder_resolution: [0, 0],
        };
        let encoder = session.init_encoder(init_params).map_err(|e| {
            NvencInitError::Failed(PreviewError::render(format!(
                "NVENC init_encoder failed: {e:?}"
            )))
        })?;

        // ── 6. allocate output bitstream + create staging texture ──
        let bitstream = encoder.create_bitstream_buffer().map_err(|e| {
            NvencInitError::Failed(PreviewError::render(format!(
                "NVENC create_bitstream failed: {e:?}"
            )))
        })?;

        let d3d = D3D11Resources::create(&device_guard.device, w, h).map_err(NvencInitError::Failed)?;

        // ── 7. register the texture ONCE (reused across all frames) ──
        // This avoids the ~5ms/frame overhead of register+unmap+unregister
        // that would otherwise dominate small-frame encoding.
        let registered = encoder
            .register_resource_dx11(&d3d.texture, NVencBufferFormat::ARGB, 0)
            .map_err(|e| {
                NvencInitError::Failed(PreviewError::render(format!(
                    "NVENC register_resource_dx11 failed: {e:?}"
                )))
            })?;

        Ok(Self {
            encoder,
            bitstream,
            annexb_buf: Vec::new(),
            frame_idx: 0,
            keyframe_period,
            d3d,
            registered,
            _device_guard: device_guard,
        })
    }
}

impl FrameEncoder for NvencEncoder {
    fn encode(&mut self, rgba: &Img) -> Result<EncodedFrame> {
        // ── update the staging texture with the new RGBA frame ──
        self.d3d.update_texture(rgba)?;

        // ── determine picture type ──
        let is_keyframe = self.frame_idx == 0 || (self.frame_idx % self.keyframe_period == 0);
        let pic_type = if is_keyframe {
            NVencPicType::IDR
        } else {
            NVencPicType::P
        };

        // ── encode (reuse the pre-registered resource) ──
        self.encoder
            .encode_picture(
                &self.registered,
                &self.bitstream,
                self.frame_idx as usize,
                self.frame_idx as u64,
                NVencBufferFormat::ARGB,
                NVencPicStruct::Frame,
                pic_type,
                None,
            )
            .map_err(|e| PreviewError::render(format!("NVENC encode_picture failed: {e:?}")))?;

        // ── read back the bitstream ──
        let bs_lock = self
            .bitstream
            .try_lock(true)
            .map_err(|e| PreviewError::render(format!("NVENC lock_bitstream failed: {e:?}")))?;
        self.annexb_buf.clear();
        self.annexb_buf.extend_from_slice(bs_lock.as_slice());
        drop(bs_lock);

        self.frame_idx += 1;

        let (sps, pps, slice) = extract_nals_from_annexb(&self.annexb_buf);
        Ok(EncodedFrame { sps, pps, slice })
    }

    fn name(&self) -> &'static str {
        "NVENC"
    }
}

// ── D3D11 device + staging texture ──

/// Holds a D3D11 device + its context alive for the lifetime of the encoder.
struct D3D11DeviceGuard {
    #[cfg(windows)]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(windows)]
    _context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
}

/// D3D11 staging texture + device context for CPU→GPU frame upload.
struct D3D11Resources {
    #[cfg(windows)]
    texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    #[cfg(windows)]
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    width: u32,
    height: u32,
}

#[cfg(windows)]
impl D3D11DeviceGuard {
    fn create() -> Result<Self> {
        use windows::Win32::Graphics::Direct3D::{
            D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
        };
        use windows::Win32::Graphics::Direct3D11::{
            D3D11CreateDevice, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
        };
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory, IDXGIAdapter, IDXGIFactory};

        // On dual-GPU systems, prefer the NVIDIA adapter so NVENC is available.
        let nvidia_adapter: Option<IDXGIAdapter> = {
            let factory: IDXGIFactory = unsafe { CreateDXGIFactory() }
                .map_err(|e| PreviewError::render(format!("CreateDXGIFactory failed: {e}")))?;
            let mut i = 0u32;
            let mut found = None;
            loop {
                match unsafe { factory.EnumAdapters(i) } {
                    Ok(adapter) => {
                        if let Ok(desc) = unsafe { adapter.GetDesc() } {
                            let name = String::from_utf16_lossy(
                                &desc.Description
                                    .iter()
                                    .take_while(|&&c| c != 0)
                                    .copied()
                                    .collect::<Vec<u16>>(),
                            );
                            if name.to_ascii_lowercase().contains("nvidia") {
                                found = Some(adapter);
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
                i += 1;
            }
            found
        };

        let adapter = nvidia_adapter.as_ref();
        let mut device = None;
        let mut context = None;
        unsafe {
            let result = D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                Default::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            );
            if result.is_err() {
                return Err(PreviewError::render(format!(
                    "D3D11CreateDevice failed: {result:?}"
                )));
            }
        }
        let device = device.ok_or_else(|| PreviewError::render("D3D11 device was null"))?;
        let context = context.ok_or_else(|| PreviewError::render("D3D11 context was null"))?;
        Ok(Self {
            device,
            _context: context,
        })
    }
}

#[cfg(windows)]
impl D3D11Resources {
    fn create(
        device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
        w: u32,
        h: u32,
    ) -> Result<Self> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
        };

        // Dynamic texture: CPU-writable via Map(WRITE_DISCARD), GPU-readable
        // for NVENC registration. This is the standard CPU→GPU upload path.
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0x10000, // D3D11_CPU_ACCESS_WRITE
            MiscFlags: 0,
        };

        let mut texture = None;
        unsafe {
            device
                .CreateTexture2D(&raw const desc, None, Some(&raw mut texture))
                .map_err(|e| PreviewError::render(format!("CreateTexture2D failed: {e}")))?;
        }
        let texture = texture.ok_or_else(|| PreviewError::render("texture was null"))?;

        // Get the immediate context for Map/Unmap operations.
        let context = unsafe { device.GetImmediateContext() }
            .map_err(|e| PreviewError::render(format!("GetImmediateContext failed: {e}")))?;

        Ok(Self {
            texture,
            context,
            width: w,
            height: h,
        })
    }

    /// Map the staging texture, memcpy RGBA into it, unmap.
    fn update_texture(&self, rgba: &Img) -> Result<()> {
        use windows::Win32::Graphics::Direct3D11::{D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD};

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(
                    &self.texture,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&raw mut mapped),
                )
                .map_err(|e| PreviewError::render(format!("D3D11 Map failed: {e}")))?;
        }
        let pitch = mapped.RowPitch as usize;
        let row_bytes = (self.width * 4) as usize;
        let data_ptr = mapped.pData as *mut u8;
        for y in 0..self.height as usize {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    rgba.data.as_ptr().add(y * row_bytes),
                    data_ptr.add(y * pitch),
                    row_bytes,
                );
            }
        }
        unsafe {
            self.context.Unmap(&self.texture, 0);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
impl D3D11DeviceGuard {
    fn create() -> Result<Self> {
        Err(PreviewError::render("NVENC is only supported on Windows"))
    }
}

#[cfg(not(windows))]
impl D3D11Resources {
    fn create(_device: &(), _w: u32, _h: u32) -> Result<Self> {
        Err(PreviewError::render("NVENC is only supported on Windows"))
    }
    fn update_texture(&self, _rgba: &Img) -> Result<()> {
        unreachable!()
    }
}
