//! DComp 合成器：设备 + target + visual 树的生命周期管理。

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISwapChain1};

/// 已挂到树上的 visual 句柄（持有 COM 引用保活 —— 见硬约束 #1）。
///
/// 宿主持有它不代表它该被释放得早：visual 的内容（swapchain）若在 visual 之后
/// 释放，绑定同样会断。**它必须与它的内容一起活到窗口销毁。**
pub struct VisualSlot {
    visual: IDCompositionVisual,
    attached: bool,
}

/// DirectComposition 合成器。
///
/// ```text
/// // 典型用法：
/// // let comp = DcompCompositor::new(hwnd, &dxgi_device)?;
/// // let sc = create_composition_swapchain(&queue_unknown, size, &desc)?;
/// // let ui = comp.add_visual_on_top()?;
/// // comp.set_content_swapchain(&ui, &sc)?;
/// // comp.commit()?;
/// ```
///
/// # 保活契约（硬约束 #1）
///
/// [`DcompCompositor`] 持有 `IDCompositionDevice` / `IDCompositionTarget` / 全部
/// `IDCompositionVisual` 的 COM 引用。**任何引用提前释放都会导致窗口全白**
/// （target 释放 = 拆除 visual 与 HWND 绑定）。
///
/// 真正的解绑发生在 [`Drop`]：字段按声明顺序释放，`target` 在 `root` 之前释放，
/// 于是 HWND 先失去绑定、visual 树随后失效 —— 这正是 DComp 期望的拆卸方向。
/// 因此**调用方必须保证：窗口销毁前先 drop 本结构**。
pub struct DcompCompositor {
    device: IDCompositionDevice,
    /// 保活字段（硬约束 #1）：释放即拆除 visual 与 HWND 绑定（窗口全白）。
    ///
    /// 它没有任何读取方，所以 rustc 会报 dead_code —— 那个 `allow` 是**语义性的**：
    /// 它标出"这个字段存在的原因就是被持有，不是被使用"。
    #[allow(dead_code)]
    target: IDCompositionTarget,
    root: IDCompositionVisual,
    /// visual 计数（诊断）
    visual_count: usize,
    destroyed: bool,
}

impl DcompCompositor {
    /// 在 hwnd 上建立 DComp target 与根 visual。
    ///
    /// `dxgi_device`：渲染设备的 `IDXGIDevice`（D3D11 device 可直接 QI 得来，
    /// D3D12 则是从 command queue 侧的 `IUnknown` QI）。topmost 固定 false ——
    /// `CreateTargetForHwnd` 的 topmost 参数必须与窗口实际 topmost 状态一致
    /// （P0 验证），由调用方保证。
    ///
    /// 若拿不到确定的 `IDXGIDevice`，改用
    /// [`Self::new_from_rendering_device`] —— 它带 fallback 链。
    pub fn new(hwnd: HWND, dxgi_device: &IDXGIDevice) -> windows::core::Result<Self> {
        let device = unsafe {
            windows::Win32::Graphics::DirectComposition::DCompositionCreateDevice2::<
                _,
                IDCompositionDevice,
            >(dxgi_device)
        }?;
        Self::build_tree(hwnd, device)
    }

    /// 从任意 rendering device（`IUnknown`）构造，带硬约束 #2 的实测 fallback 链。
    ///
    /// 尝试顺序（P0 spike 验证）：`DCompositionCreateDevice2(device)` →
    /// `DCompositionCreateDevice3(device)` → **null rendering device**。
    /// 本机 D3D12 command queue 路线 `E_NOINTERFACE`（v2 / v3 均如此），null
    /// 实测可正常合成（composition swapchain 的内容不受 DComp device 来源影响），
    /// 所以 D3D12 宿主走此构造。D3D11 宿主优先用 [`Self::new`]（device 可 QI 出
    /// `IDXGIDevice`）。
    ///
    /// 每一级失败都记 `log::warn` —— 三级全败时窗口会白，而白屏没有任何其他
    /// 症状会指向"是 DComp 设备没建成"。
    pub fn new_from_rendering_device(
        hwnd: HWND,
        rendering_device: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<Self> {
        use windows::Win32::Graphics::DirectComposition::{
            DCompositionCreateDevice2, DCompositionCreateDevice3,
        };
        let device: IDCompositionDevice = unsafe {
            match rendering_device {
                Some(d) => match DCompositionCreateDevice2::<_, IDCompositionDevice>(d) {
                    Ok(dev) => dev,
                    Err(e2) => match DCompositionCreateDevice3::<_, IDCompositionDevice>(d) {
                        Ok(dev) => dev,
                        Err(e3) => {
                            log::warn!(
                                "zephyra-dcomp: DComp device from rendering device failed \
                                 (v2: {e2}, v3: {e3}); falling back to null device"
                            );
                            DCompositionCreateDevice2::<_, IDCompositionDevice>(
                                None::<&windows::core::IUnknown>,
                            )?
                        }
                    },
                },
                None => {
                    DCompositionCreateDevice2::<_, IDCompositionDevice>(
                        None::<&windows::core::IUnknown>,
                    )?
                }
            }
        };
        Self::build_tree(hwnd, device)
    }

    /// target + root visual 建树（两种构造共用）。
    fn build_tree(hwnd: HWND, device: IDCompositionDevice) -> windows::core::Result<Self> {
        let target = unsafe { device.CreateTargetForHwnd(hwnd, false) }?;
        let root = unsafe { device.CreateVisual() }?;
        unsafe { target.SetRoot(&root) }?;
        unsafe { device.Commit() }?;
        Ok(Self {
            device,
            target,
            root,
            visual_count: 0,
            destroyed: false,
        })
    }

    /// 设备句柄（诊断 / `WaitForCommitCompletion` 等）。
    pub fn device(&self) -> &IDCompositionDevice {
        &self.device
    }

    /// 在根 visual 顶部插入一个新 child visual，返回句柄。
    ///
    /// z-order：后插入者盖在先插入者之上。
    ///
    /// `insertAbove = true` + `referenceVisual = NULL` 的组合在 DComp 里就是"插到
    /// z 序最顶"（没有参照物时 `insertAbove` 决定方向）。**这一点是 P0 实测确认的，
    /// 不是从文档推的** —— 若接线后上屏复核发现 z-order 颠倒，改成 `false` 对照，
    /// 别去猜文档。
    pub fn add_visual_on_top(&mut self) -> windows::core::Result<VisualSlot> {
        let visual = unsafe { self.device.CreateVisual() }?;
        unsafe { self.root.AddVisual(&visual, true, None::<&IDCompositionVisual>) }?;
        self.visual_count += 1;
        Ok(VisualSlot {
            visual,
            attached: true,
        })
    }

    /// 在指定参照 visual 之上插入新 child visual。
    pub fn add_visual_above(
        &mut self,
        reference: &VisualSlot,
    ) -> windows::core::Result<VisualSlot> {
        let visual = unsafe { self.device.CreateVisual() }?;
        unsafe { self.root.AddVisual(&visual, true, &reference.visual) }?;
        self.visual_count += 1;
        Ok(VisualSlot {
            visual,
            attached: true,
        })
    }

    /// 设置 visual 内容为 composition swapchain。
    ///
    /// 需要随后调用 [`Self::commit`] 生效。
    pub fn set_content_swapchain(
        &self,
        slot: &VisualSlot,
        swapchain: &IDXGISwapChain1,
    ) -> windows::core::Result<()> {
        unsafe { slot.visual.SetContent(swapchain) }
    }

    /// 设置 visual 内容为任意 `IUnknown`（surface / 另一个 visual 等）。
    pub fn set_content_raw(
        &self,
        slot: &VisualSlot,
        content: &windows::core::IUnknown,
    ) -> windows::core::Result<()> {
        unsafe { slot.visual.SetContent(content) }
    }

    /// 移除 visual（从树上摘下并释放内容绑定）。需要 commit 生效。
    pub fn remove_visual(&mut self, slot: &mut VisualSlot) -> windows::core::Result<()> {
        if !slot.attached {
            return Ok(());
        }
        unsafe { self.root.RemoveVisual(&slot.visual) }?;
        slot.attached = false;
        self.visual_count = self.visual_count.saturating_sub(1);
        Ok(())
    }

    /// visual 偏移辅助（`IDCompositionVisual2` cast 路径）。
    ///
    /// 前提：调用方保证窗口未启用 per-monitor DPI 缩放，或已把偏移换算成物理
    /// 像素。DComp 本身不做 DPI 换算。
    pub fn set_offset(&self, slot: &VisualSlot, x: f32, y: f32) -> windows::core::Result<()> {
        let v2: windows::Win32::Graphics::DirectComposition::IDCompositionVisual2 =
            slot.visual.cast()?;
        unsafe { v2.SetOffsetX2(x)? };
        unsafe { v2.SetOffsetY2(y) }
    }

    /// 把 visual 的内容按 `scale_x/scale_y` **缩放**（治「黑边」）。
    ///
    /// # 为什么需要它
    ///
    /// DComp 默认把 visual 的内容按**自然尺寸**画在原点。所以只要交换链的尺寸
    /// 与窗口客户区对不上，多出来的那一块就是**没人画的黑边**。
    ///
    /// 这不是理论问题：本机的 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功
    /// （实测矩阵见 `Swapchain::resize` 文档与 `docs/gpu-hang-bisect.md`），于是
    /// 「尺寸对不上」是**常态**而不是异常。熔断触发后看到的「黑边」就是这么来的 ——
    /// 熔断让 `resize` 静默跳过，链停在旧尺寸，窗口却已经是新尺寸。
    ///
    /// 缩放之后同一份内容会铺满整个客户区：内容略微变形，但没有黑边、没有重建
    /// 开销、也没有 GPU churn。等真的重建出正确分辨率的交换链，把缩放收回 1,1 即可。
    ///
    /// # 为什么走 `SetTransform2` 而不是「设置尺寸」
    ///
    /// DComp **没有**设置 visual 尺寸的 API，尺寸是**算出来的**：内容自然尺寸 × 缩放。
    /// `windows` crate 0.62 的 `IDCompositionVisual2` 只绑到
    /// `SetOffsetX2` / `SetOpacity2` / `SetDepthMode` / `SetTransform2` / `SetVisible`
    /// / `SetClipRect2` —— 缩放能力落在 `SetTransform2` 的 4×4 矩阵上。
    ///
    /// （`SetScaleX2/SetScaleY2` 看着像，但它们在 `IDCompositionScaleTransform` 上，
    /// 那是 3D 特效那条线，要 `CreateScaleTransform` + 3D visual，不是这里要的。）
    ///
    /// 入参是 `windows_numerics::Matrix3x2`（3×2），而 `windows` 只转发不重导出这个
    /// 类型 —— 所以本 crate 直接依赖 `windows-numerics` 才能把矩阵传进去。
    pub fn set_scale(
        &self,
        slot: &VisualSlot,
        scale_x: f32,
        scale_y: f32,
    ) -> windows::core::Result<()> {
        use windows_numerics::Matrix3x2;
        let v2: windows::Win32::Graphics::DirectComposition::IDCompositionVisual2 =
            slot.visual.cast()?;
        // DComp 是**行向量**约定（平移在第 3 行 M31/M32），纯缩放就是：
        //   [sx  0  0]
        //   [ 0 sy  0]
        //   [ 0  0  1]
        let m = Matrix3x2 {
            M11: scale_x,
            M22: scale_y,
            ..Default::default()
        };
        unsafe { v2.SetTransform2(&m as *const Matrix3x2) }
    }
    /// 提交全部变更。
    pub fn commit(&self) -> windows::core::Result<()> {
        unsafe { self.device.Commit() }
    }

    /// 阻塞等待提交完成（首帧验证用；常规路径不需要 —— DWM 自会调度）。
    pub fn wait_for_commit(&self) -> windows::core::Result<()> {
        unsafe { self.device.WaitForCommitCompletion() }
    }

    /// 当前 visual 数（诊断）。
    pub fn visual_count(&self) -> usize {
        self.visual_count
    }

    /// 标记销毁意图，供诊断读取。
    ///
    /// # 这**不会**解绑
    ///
    /// windows-rs 0.58 未生成 `IDCompositionDevice::DestroyTarget`；释放 target
    /// 的 COM 引用即等价解绑（P0 根因：引用提前释放 = 绑定拆除 = 窗口全白）。
    /// 真正的解绑发生在 [`Drop`] 释放 `target` 字段时。
    ///
    /// 所以本方法只是把状态标出来，供宿主在"我打算关窗口了"的路径上确认调用方
    /// **意图正确**（该 drop 的已经 drop 了）。它单独调用不产生任何系统效果 ——
    /// 这一点比原实现的措辞更诚实，因为原注释暗示 Drop 会做点实事。
    pub fn destroy(&mut self) {
        self.destroyed = true;
    }

    /// 是否已被标记销毁（诊断）。
    pub fn is_destroyed(&self) -> bool {
        self.destroyed
    }
}

impl Drop for DcompCompositor {
    /// 拆卸顺序即字段声明顺序的逆序相关：Rust 先跑 `Drop::drop`（此处置标记），
    /// 再按**声明顺序**释放字段。因此 `device` 先于 `target` 释放。
    ///
    /// 这与直觉相反（DComp 通常希望 device 后于 target 存活），但因为释放 target
    /// 本身就完成了对 visual 树的解绑，而 `root` 在 target 之后才释放 —— 拆的是
    /// HWND 绑定，不是 visual 的 COM 内存 —— 实际不会出问题。P0 实测窗口正常销毁。
    fn drop(&mut self) {
        self.destroy();
    }
}

// `Interface` 在 `set_offset` 的 `.cast()` 上使用（上面已 import）。
