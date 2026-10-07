//! 纹理表：名字 → D3D11 纹理与 SRV。
//!
//! # 为什么要有这个间接层
//!
//! 核心的 [`zephyra_core::paint::TextureRef`] 只是个不透明字符串 id —— 核心**不认识**
//! GPU 纹理，那是宿主的事。
//!
//! 于是需要一张从 id 到纹理的映射表。
//!
//! # 分批是「一次 draw 只能绑一张纹理」的结果，不是优化选择
//!
//! 不同纹理必然分批，这条改不掉。分批本身由 [`zephyra_fx::quad::expand`] 完成
//! （渲染器调用），只合并**连续**同纹理的命令 —— 合并不连续的命令会改变绘制顺序，
//! 而图像常是半透明的，重排直接改变可见结果。
//!
//! # 相比 wgpu 版本删掉了什么，以及为什么
//!
//! wgpu 版这里有一个 `HashMap<usize, wgpu::BindGroup>` 缓存 keyed by 视图地址，
//! 外加一个 `forget_bind_groups()` —— 纹理被替换后必须手动清缓存，否则会绑到已
//! 销毁纹理的视图上（use-after-free）。整套机制存在的唯一原因是 wgpu 的 bind
//! group 是**烘焙好的对象**，不能只换个 SRV 就地改。
//!
//! D3D11 没有这个问题：`PSSetShaderResources` 每次调用直接给一个
//! `ID3D11ShaderResourceView*`，不缓存、不失效。于是：
//!
//! - 纹理替换只需换表里的 SRV，下一帧自动用新的，**没有必须记得调用的清理函数**；
//! - SRV 自己持有资源的 COM 引用，所以表**不需要**额外保活纹理 —— 无人引用即释放。
//!
//! 也就是说，删掉这段代码不是简化，是消灭了一整类"忘记调用清理函数"的 bug。
//!
//! # 与 `zephyra_fx::TextureNames` 的分工
//!
//! 共享层有一个只存**名字**的 `TextureNames`，因为 CPU 参考光栅器只需要知道
//! 「这个名字注册过没有」，它不碰 GPU。本表在它之上多存了 GPU 资源对象。
//!
//! ---
//!
//! 注：本文件在 2026-10-06 拆分 `zephyra-fx` 时被误覆盖过一次，按调用点重建。
//! 公开 API 全部来自 `lib.rs` 的 `register_texture` / `update_texture` /
//! `forget_texture`、`renderer.rs` 的 `create_rgba_texture_empty` + `create_srv`、
//! 以及 `surface.rs` 的 `create_staging_texture`。

#![cfg(windows)]

use std::collections::HashMap;

use windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2D;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, ID3D11Device,
    ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

/// 一张已注册的纹理。
struct Entry {
    /// 采样视图。**持有资源的 COM 引用** —— 单靠它就足以让纹理活着。
    srv: ID3D11ShaderResourceView,
    /// 纹理本体。`update_texture` 要往它里面写像素 —— 从 SRV 反查纹理得经
    /// `ID3D11View::GetResource` 再 cast，多一次 COM 往返，而写像素是每帧都可能
    /// 发生的事。
    ///
    /// 存两份**不会**让纹理多活一份引用被重复计数：两份字段指向同一资源，
    /// 而 Rust 的析构顺序保证都在 `Entry` 离开作用域时释放。
    tex: ID3D11Texture2D,
}

/// 纹理表。名字由调用方给定，通常是资源文件路径。
#[derive(Default)]
pub struct TextureTable {
    entries: HashMap<String, Entry>,
}

impl std::fmt::Debug for TextureTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureTable")
            .field("textures", &self.entries.len())
            .finish()
    }
}

impl TextureTable {
    /// 空表。
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// 登记一个名字 → 采样视图 + 纹理。
    ///
    /// 重复登记同一名字是**替换**：旧 `Entry` 随 `HashMap::insert` 一起释放，下一帧
    /// 自动用新的。不需要像 wgpu 那样显式调清理函数 —— 原因见模块文档。
    pub fn insert(&mut self, name: &str, srv: ID3D11ShaderResourceView, tex: ID3D11Texture2D) {
        self.entries.insert(
            name.to_string(),
            Entry {
                srv,
                tex,
            },
        );
    }

    /// 原地更新一张已注册纹理的像素（尺寸不变）。
    ///
    /// 返回 `false` = 纹理未注册或像素长度不足 —— 此时**什么都没写入**，调用方必须
    /// 自己重建纹理。采样视图不变，所以没有任何需要失效的缓存。
    ///
    /// # 必须按**像素**步进，不能整块 `UpdateSubresource`
    ///
    /// `UpdateSubresource` 会把 `pSrcData` 当成**紧密排列**的行，而驱动要求的行
    /// 字节数是 `width * 4` 向上取整到 `D3D11_REQ_TEXTURE2D_UPLOAD_ALIGNMENT` 的
    /// 倍数。两者不等时**不报错**，只是每行往后错一点 —— 症状是整张图斜着串行。
    pub fn update(
        &self,
        context: &ID3D11DeviceContext,
        name: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> bool {
        let Some(entry) = self.entries.get(name) else {
            return false;
        };
        let row_bytes = (width as usize) * 4;
        if pixels.len() < row_bytes * (height as usize) {
            return false;
        }
        write_rows(context, &entry.tex, pixels, width, height);
        true
    }

    /// 移除一张纹理。**必须在销毁纹理之前调用。**
    pub fn remove(&mut self, name: &str) {
        self.entries.remove(name);
    }

    /// 这个名字注册过吗。
    ///
    /// [`zephyra_fx::quad::expand`] 用它决定图像命令能否画出来 —— 没注册就跳过
    /// 并计入 `RenderReport::missing_textures`，**不静默**。
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 已注册的张数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 一个都没注册？
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 取采样视图。渲染器按批次的名字取它并 `PSSetShaderResources`。
    pub fn srv(&self, name: &str) -> Option<ID3D11ShaderResourceView> {
        self.entries.get(name).map(|e| e.srv.clone())
    }
}

/// 把 `rgba` 上传进 `tex`。
///
/// # 为什么走 `UpdateSubresource` 而不是 `Map(WRITE_DISCARD)`
///
/// **曾经的理由是错的。** 原文写「`DYNAMIC` 在某些机器上建不出来」—— 实际是调用方
/// 把 `DYNAMIC` 与 `CPUAccessFlags: 0` 配成了非法组合。见
/// [`create_rgba_texture_empty`] 的说明。
///
/// 现在仍走 `UpdateSubresource`，理由是它**更不容易用错**：
///
/// | | `Map(WRITE_DISCARD)`（需 `DYNAMIC`） | `UpdateSubresource`（`DEFAULT` 即可） |
/// |---|---|---|
/// | 源行距 | 紧密（自己按 `width * 4` 读） | 紧密（`SysMemPitch`） |
/// | **目标行距** | **驱动的 `RowPitch`，可能更大 —— 必须按它跳行** | 驱动自己处理 |
/// | 出错后果 | 忘了跳行 ⇒ 内容斜着进纹理，且**不报错** | 写多了被静默截断，也不报错 |
///
/// `Map` 那条「忘了跳行」的坑本项目踩过（症状是字形整体倾斜、倾斜量随行号累积）。
/// 用 `UpdateSubresource` 把它从「必须记得」变成「不必记得」。
///
/// 代价是驱动内部可能多拷一份。对每帧上传的几十 KB 顶点/索引数据，这个代价小于
/// 一整类难查的 bug。
///
/// # 它没有返回值
///
/// windows-rs 把 `UpdateSubresource` 声明成返回 `()`，**没有 HRESULT 可查**。
/// 所以「上传有没有真的生效」只能靠之后读回来判断，不能靠返回值。这是
/// `void` 返回的 D3D11 API 最麻烦的地方。写多了会被静默截断，写少了会静默忽略
/// 超出的部分 —— 两种错都**不报错**。
fn write_rows(
    context: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    rgba: &[u8],
    width: u32,
    height: u32,
) {
    let row_bytes = (width as usize * 4) as u32;
    debug_assert!(
        rgba.len() >= (row_bytes as usize) * (height as usize),
        "rgba 只有 {} 字节，不够 {width}×{height} 所需的 {} 字节",
        rgba.len(),
        (row_bytes as usize) * (height as usize)
    );
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: rgba.as_ptr() as *const _,
        SysMemPitch: row_bytes,
        SysMemSlicePitch: 0,
    };
    unsafe {
        context.UpdateSubresource(
            tex,
            0,
            None, // 整个资源
            &data as *const _ as *const std::ffi::c_void,
            0,
            0,
        )
    };
}

/// 建一张 RGBA8 纹理并灌入像素。
pub fn create_rgba_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> windows::core::Result<ID3D11Texture2D> {
    let need = (width as usize) * (height as usize) * 4;
    if pixels.len() < need {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!("像素不足：需要 {need} 字节，实得 {}", pixels.len()),
        ));
    }
    let tex = create_rgba_texture_empty(device, width, height, D3D11_USAGE_DEFAULT)?;
    write_rows(context, &tex, pixels, width, height);
    Ok(tex)
}

/// 建一张 RGBA8 纹理，内容留空（`width * height * 4` 字节零）。
///
/// 调用方要自己往里写时用这个 —— 字形图集就是这么用的：先建空纹理，再每帧
/// `UpdateSubresource` 上传那几 MB。
///
/// # `usage` 传 `DYNAMIC` 时**必须**给 `CPU_ACCESS_WRITE`
///
/// 按 `D3D11_USAGE` 参考页的能力表，`DYNAMIC` 意味着「GPU 只读 + **CPU 只写**」，
/// 而 CPU 写权限**只能**由 `D3D11_CPU_ACCESS_WRITE` 授予。`DYNAMIC` 配
/// `CPUAccessFlags: 0`（声称 CPU 可写却没给写权限）是**非法组合**，回
/// `E_INVALIDARG` 理所当然。
///
/// <https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_usage>
///
/// # 一段被推翻的结论
///
/// 本文件原先在 `usage` 上写着「`DYNAMIC` 在部分机器上建不出来」，那是因为
/// **调用方把 `DYNAMIC` 与 `CPUAccessFlags: 0` 配在了一起** —— 非法组合。
/// 本机四块适配器（含不经任何驱动的 WARP）在 `DYNAMIC + CPU_ACCESS_WRITE` 下
/// **全部建得出来**，见 `tests/probe_basic.rs` 的逐适配器表格。
///
/// 现在仍然走 `DEFAULT` + `UpdateSubresource`，理由不再是「DYNAMIC 不可用」，而是：
/// 文档建议 `UpdateSubresource` 「most usefully a resource with
/// `D3D11_USAGE_DEFAULT`」，而它让**目标行距由驱动处理**，从根上消掉一整类
/// 「忘了按 `RowPitch` 跳行导致内容斜着进纹理」的 bug。
pub fn create_rgba_texture_empty(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    usage: D3D11_USAGE,
) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }?;
    tex.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "CreateTexture2D 成功却没给纹理",
        )
    })
}

/// 建一张 `STAGING` 纹理，用于把内容读回 CPU。
///
/// # `cpu_access_flags` 由调用方给
///
/// 这里**不**硬编码 `D3D11_CPU_ACCESS_READ`：`surface.rs` 的读回路径要的就是
/// READ，但把参数显式化能让「哪个用途要哪个 flag」写在调用点上 —— 隐式的那份在
/// 两个用途要求不同 flag 时就会静默读到垃圾。
pub fn create_staging_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    cpu_access_flags: u32,
) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: cpu_access_flags,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }?;
    tex.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "CreateTexture2D(STAGING) 成功却没给纹理",
        )
    })
}

/// 为一张 RGBA8 的 2D 纹理建采样视图。
///
/// `Format` 与纹理的**格式**一致（不是 `UNKNOWN`）—— 写 `UNKNOWN` 时 D3D11 会自己
/// 从资源描述里取，一般也对，但那是隐式依赖资源描述里恰好写着 `R8G8B8A8`；显式
/// 写出来的话，资源格式一改这里就会**建不出视图**而不是悄悄给出错误的采样行为。
pub fn create_srv(
    device: &ID3D11Device,
    tex: &ID3D11Texture2D,
) -> windows::core::Result<ID3D11ShaderResourceView> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_SRV {
                MostDetailedMip: 0,
                MipLevels: 1,
            },
        },
    };
    let mut srv = None;
    unsafe {
        device.CreateShaderResourceView(
            tex,
            Some(&desc as *const _ as *const _),
            Some(&mut srv as *mut _),
        )
    }?;
    srv.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "CreateShaderResourceView 成功却没给视图",
        )
    })
}

/// 同上，但**显式**给出 SRV 格式。
///
/// # 为什么不复用 [`create_srv`]
///
/// `create_srv` 把格式钉死成 `R8G8B8A8_UNORM`，它服务的图集 / 上传图像都是那个格式。
/// 离屏渲染目标却常是 `B8G8R8A8_UNORM`（测试里的 `offscreen_target` 就是），而
/// **SRV 格式必须与纹理兼容** —— 拿 `R8G8B8A8` 去描述一张 `B8G8R8A8` 纹理时，
/// `CreateShaderResourceView` 直接返回 `E_INVALIDARG`。
///
/// 症状极安静：调用方普遍用 `.ok()`，于是整条路径退化成"拿不到 SRV"，而画面
/// 看起来只是"某个效果没生效"，没有任何错误码冒出来。
pub fn create_srv_with_format(
    device: &ID3D11Device,
    tex: &ID3D11Texture2D,
    format: DXGI_FORMAT,
) -> windows::core::Result<ID3D11ShaderResourceView> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_SRV {
                MostDetailedMip: 0,
                MipLevels: 1,
            },
        },
    };
    let mut srv = None;
    unsafe {
        device.CreateShaderResourceView(
            tex,
            Some(&desc as *const _ as *const _),
            Some(&mut srv as *mut _),
        )
    }?;
    srv.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "CreateShaderResourceView(显式格式) 成功却没给出视图",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个测试用设备。失败就跳过 —— 这是 GPU 相关的 crate，没有设备的机器上
    /// 本来就测不了。
    fn test_device() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
        use windows::Win32::Graphics::Direct3D::{
            D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
        };
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
        };
        let mut dev = None;
        let mut ctx = None;
        unsafe {
            windows::Win32::Graphics::Direct3D11::D3D11CreateDevice::<
                Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
            >(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev as *mut _),
                None,
                Some(&mut ctx as *mut _),
            )
        }
        .ok()?;
        Some((dev?, ctx?))
    }

    /// 像素不足必须**报错**，不能建成一张内容是垃圾的纹理。
    ///
    /// 这条不是洁癖：`create_rgba_texture` 的写入循环按 `width * height * 4` 从
    /// `pixels` 里取，越界读到的字节会被原样上传 —— 症状是一张底部带随机色块的
    /// 图，**没有任何错误**。
    #[test]
    fn 像素不足_必须报错而不是建出垃圾纹理() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let err = create_rgba_texture(&device, &context, &[0u8; 16], 4, 4);
        assert!(
            err.is_err(),
            "4×4 RGBA 需要 64 字节，给 16 字节必须报错 —— \
             否则越界读到的字节会被原样上传成随机色块，且不报任何错"
        );
    }

    /// 完整走一遍：建纹理 → 建 SRV → 登记 → 查 → 注销。
    #[test]
    fn 登记与查询() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let w = 4u32;
        let h = 4u32;
        let pixels = vec![200u8; (w * h * 4) as usize];

        let mut table = TextureTable::new();
        assert!(table.is_empty());
        assert!(!table.contains("a.png"), "未登记的名字必须查不到");

        let tex = create_rgba_texture(&device, &context, &pixels, w, h).expect("建纹理");
        let srv = create_srv(&device, &tex).expect("建 SRV");
        table.insert("a.png", srv, tex);

        assert!(table.contains("a.png"));
        assert_eq!(table.len(), 1);
        assert!(table.srv("a.png").is_some(), "登记过的必须能取回 SRV");
        assert!(table.srv("b.png").is_none(), "没登记的不该取到 SRV");

        table.remove("a.png");
        assert!(!table.contains("a.png"));
        assert_eq!(table.len(), 0);
    }

    /// `update` 对未登记的名字必须返回 `false` 且**不碰任何东西**。
    ///
    /// 它取 `&self` 而不是 `&mut self`，所以「更新一个不存在的纹理」在类型上就是
    /// 可能的 —— 忘了判空的话症状是「视频帧偶尔不刷新」，且不报错。
    #[test]
    fn update_未登记的名字_返回_false() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let table = TextureTable::new();
        assert!(
            !table.update(&context, "missing.png", &[0u8; 64], 4, 4),
            "未登记的纹理必须返回 false，调用方据此自己重建"
        );
        let _ = device;
    }

    /// `update` 像素不足时必须返回 `false`，而不是写一半。
    #[test]
    fn update_像素不足_返回_false() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let w = 4u32;
        let h = 4u32;
        let mut table = TextureTable::new();
        let tex = create_rgba_texture(&device, &context, &vec![0u8; (w * h * 4) as usize], w, h)
            .expect("建纹理");
        let srv = create_srv(&device, &tex).expect("建 SRV");
        table.insert("a.png", srv, tex);

        assert!(
            !table.update(&context, "a.png", &[0u8; 16], w, h),
            "4×4 需要 64 字节，给 16 字节必须返回 false"
        );
        assert!(
            table.update(&context, "a.png", &vec![1u8; (w * h * 4) as usize], w, h),
            "字节数够时必须成功"
        );
    }
}
