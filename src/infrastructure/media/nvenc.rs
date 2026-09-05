//! NVIDIA NVENC H.264 编码后端。
//!
//! 通过 `nvenc` crate（内部使用 `libloading`）在运行时动态加载
//! `nvEncodeAPI64.dll`。DLL 缺失（没有 NVIDIA GPU / 驱动）时，
//! `try_create` 返回 `Ok(None)`，调用方继续尝试下一个后端。
//!
//! ## 输入路径
//!
//! 使用 **D3D11 staging texture**（可由 CPU 写入）作为 NVENC 输入源。
//! 每帧执行：Map 纹理 → memcpy RGBA → Unmap → `register_resource_dx11`
//! → `encode_picture` → drop（自动 unmap + unregister）。纹理只创建一次并在帧间复用。
//!
//! 由于 crate 的 `InputBufferLock::drop` 存在 bug（向 `unlock_input_buffer`
//! 传入 `buffer_data_ptr` 而不是 buffer handle），此路径替代了
//! `create_input_buffer` + `InputBuffer::lock()`。`register_resource_dx11` 返回的
//! `RegisteredResource` 包装器实现了正确的 Drop，因此可以安全使用。
//!
//! ## 配置
//!
//! - H.264，最快 P1 预设、LowLatency 调优、900 kbps VBR、无 B 帧。
//! - GOP 为 2 秒帧数；NVENC 默认在首个 IDR 前输出 SPS/PPS。
//! - 输出为 Annex-B，由共享 `mux` 模块解析。

use crate::domain::errors::{PreviewError, Result};
use crate::render::canvas::Img;

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};

use nvenc::bitstream::BitStream;
use nvenc::session::{InitParams, Session};
use nvenc::sys::enums::{
    NVencBufferFormat, NVencParamsRcMode, NVencPicStruct, NVencPicType, NVencTuningInfo,
};
use nvenc::sys::guids::{NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P1_GUID};

/// 尝试创建 NVENC 编码器。NVENC DLL 不可用或会话创建失败
///（例如没有 NVIDIA GPU）时返回 `Ok(None)`。
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
    /// 复用的 Annex-B 拼接缓冲区。
    annexb_buf: Vec<u8>,
    frame_idx: u32,
    keyframe_period: u32,
    /// D3D11 设备与 staging texture（可由 CPU 写入，初始化时向 NVENC 注册一次并在所有帧中复用）。
    d3d: D3D11Resources,
    /// 已注册的 NVENC 输入资源，仅创建一次并在每帧复用，
    /// 避免每帧 register + unmap + unregister 约 5ms 的开销。
    registered: nvenc::encoder::RegisteredResource,
    _device_guard: D3D11DeviceGuard,
}

unsafe impl Send for NvencEncoder {}

impl NvencEncoder {
    fn new(w: u32, h: u32, fps: u32) -> std::result::Result<Self, NvencInitError> {
        if !w.is_multiple_of(2) || !h.is_multiple_of(2) {
            return Err(NvencInitError::Failed(PreviewError::render(format!(
                "NVENC requires even dimensions, got {w}x{h}"
            ))));
        }

        // ── 1. 创建设备（NVENC 会话 + staging texture） ──
        let device_guard = D3D11DeviceGuard::create().map_err(NvencInitError::Failed)?;

        // ── 2. 打开 NVENC 会话 ──
        let session = match Session::open_dx(&device_guard.device) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[video] NVENC open_dx failed: {e:?}");
                return Err(NvencInitError::Unavailable);
            }
        };

        // ── 3. 获取预设配置 ──
        let (session, mut config) = session
            .get_encode_preset_config_ex(
                NV_ENC_CODEC_H264_GUID,
                NV_ENC_PRESET_P1_GUID,
                NVencTuningInfo::LowLatency,
            )
            .map_err(|e| {
                NvencInitError::Failed(PreviewError::render(format!(
                    "NVENC get_preset_config failed: {e:?}"
                )))
            })?;

        // ── 4. 优化吞吐量和输出大小 ──
        let keyframe_period = (fps * 2).max(1);
        config.preset_cfg.gop_len = keyframe_period;
        config.preset_cfg.frame_interval_p = 1;
        config.preset_cfg.rc_params.rate_control_mode = NVencParamsRcMode::VBR;
        config.preset_cfg.rc_params.average_bit_rate = crate::infrastructure::config::current()
            .advance
            .video
            .VIDEO_BITRATE;

        // ── 5. 初始化编码器 ──
        let init_params = InitParams {
            encode_guid: NV_ENC_CODEC_H264_GUID,
            preset_guid: NV_ENC_PRESET_P1_GUID,
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

        // ── 6. 分配输出 bitstream 并创建 staging texture ──
        let bitstream = encoder.create_bitstream_buffer().map_err(|e| {
            NvencInitError::Failed(PreviewError::render(format!(
                "NVENC create_bitstream failed: {e:?}"
            )))
        })?;

        let d3d =
            D3D11Resources::create(&device_guard.device, w, h).map_err(NvencInitError::Failed)?;

        // ── 7. 仅注册一次纹理（所有帧复用） ──
        // 避免 register + unmap + unregister 每帧约 5ms 的开销，
        // 否则该开销会主导小帧编码时间。
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
        // ── 使用新的 RGBA 帧更新 staging texture ──
        self.d3d.update_texture(rgba)?;

        // ── 确定图像类型 ──
        let is_keyframe =
            self.frame_idx == 0 || self.frame_idx.is_multiple_of(self.keyframe_period);
        let pic_type = if is_keyframe {
            NVencPicType::IDR
        } else {
            NVencPicType::P
        };

        // ── 编码（复用预注册资源） ──
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

        // ── 读回 bitstream ──
        let bs_lock = self
            .bitstream
            .try_lock(true)
            .map_err(|e| PreviewError::render(format!("NVENC lock_bitstream failed: {e:?}")))?;
        self.annexb_buf.clear();
        self.annexb_buf.extend_from_slice(bs_lock.as_slice());
        drop(bs_lock);

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
        "NVENC"
    }
}

// ── D3D11 设备与 staging texture ──

/// 在编码器生命周期内保持 D3D11 设备及其上下文存活。
struct D3D11DeviceGuard {
    #[cfg(windows)]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(windows)]
    _context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
}

/// 用于 CPU→GPU 帧上传的 D3D11 staging texture 与设备上下文。
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
        use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
        use windows::Win32::Graphics::Direct3D11::{
            D3D11CreateDevice, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
        };
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory, IDXGIAdapter, IDXGIFactory};

        // 双 GPU 系统优先选择 NVIDIA 适配器，以确保 NVENC 可用。
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
                                &desc
                                    .Description
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

        // 动态纹理：通过 Map(WRITE_DISCARD) 由 CPU 写入，并可供 GPU 读取以注册到 NVENC。
        // 这是标准的 CPU→GPU 上传路径。
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

        // 获取用于 Map/Unmap 操作的 immediate context。
        let context = unsafe { device.GetImmediateContext() }
            .map_err(|e| PreviewError::render(format!("GetImmediateContext failed: {e}")))?;

        Ok(Self {
            texture,
            context,
            width: w,
            height: h,
        })
    }

    /// Map staging texture，将 RGBA memcpy 进去，再执行 unmap。
    fn update_texture(&self, rgba: &Img) -> Result<()> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
        };

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
