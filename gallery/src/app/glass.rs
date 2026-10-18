// ------------------------------------------------------- 液态玻璃的折射参数

/// 折射的四个可调参数 —— 全部以**档位下标**存储，值与显示文本都查表。
///
/// # 为什么存下标而不是 `f32`
///
/// `view()` 每帧重建整棵树，运行期 `format!("{:.2}", v)` 会走 `Box::leak`
/// 每帧一次（见 `pages::basics::spin_text` 的理由）。而这一页的参数是要
/// **每帧读**的（画在玻璃块上、也画在参数面板上）。
///
/// 存下标让「数值」与「文本」都变成一次数组查表 —— 零分配、零泄漏，
/// 而且**加减只是 `±1` 后 clamp**，不需要浮点误差累积。
///
/// # 为什么不是「四个自由 f32」
///
/// 那样步进要写 `(v + 0.05).min(1.0)`，而浮点累加在边界上会漂
/// （0.05 加 20 次到 1.0000001，然后 `.min(1.0)` 与不等号判据打架）。
/// 档位把这件事变成整数运算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glass {
    /// 玻璃**厚度**（像素）档位，见 [`THICKNESS`]。
    pub thickness: usize,
    /// 倒角带**宽度**（像素）档位，见 [`BEZEL`]。
    pub bezel: usize,
    /// **色散**量档位，见 [`DISPERSION`]。
    pub dispersion: usize,
    /// **折射率**（IOR）档位，见 [`IOR`]。
    pub ior: usize,
}

impl Default for Glass {
    /// 默认值取「普通冕牌玻璃」：厚 28px、倒角 26px、几乎不色散、IOR 1.50。
    ///
    /// 为什么不取全 0：全 0 时 `strength = 0` 会让整条滤镜链**恒等**，
    /// 于是玻璃块根本不进离屏 pass —— 那一帧看起来像「折射没接上」，
    /// 而真正的原因只是「厚度调成 0 了」。默认给一个能看见效果的组合。
    fn default() -> Self {
        Self {
            thickness: 7,
            bezel: 12,
            dispersion: 4,
            ior: 10,
        }
    }
}

impl Glass {
    /// 当前档位对应的 `Filter::Refraction`。
    ///
    /// # 为什么这里可以带**数值**而其它属性不行
    ///
    /// 本仓的规矩是「外观不进源码」，门禁 `the_whole_tree_writes_no_appearance_in_source`
    /// 按 `iter_local()` 逐个属性查。它列的 19 个 APPEARANCE 里**没有 `FILTER`** ——
    /// 而这是对的：
    ///
    /// `FILTER` 的四个参数是**物理量**（玻璃厚度、折射率、色散、倒角带宽度），
    /// 不是**样式**（颜色、间距、圆角）。它们没有 `@令牌` 对应物，也不会有
    /// 主题版本 —— 一块玻璃有多厚不取决于界面是深色还是浅色。
    ///
    /// 更关键的是**这个 demo 的职责就是让人调它们**。参数写进 `.zstyle`
    /// 就只剩三档静态值，「折射允许调参数」直接不成立。
    ///
    /// `PropertyValue::Filter` 参与 `PartialEq`（`property.rs` 的相等表里有一项），
    /// 所以改参数能被 `diff` 正确识别为「这棵树变了」——不需要额外的失效通道。
    pub fn filter(&self) -> zephyra_core::filter::Filter {
        zephyra_core::filter::Filter::Refraction {
            strength: THICKNESS[self.thickness],
            bezel: BEZEL[self.bezel],
            dispersion: DISPERSION[self.dispersion],
            ior: IOR[self.ior],
        }
    }

    /// 四个参数各自的显示文本（查表，见 [`Glass`] 的理由）。
    pub fn texts(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        (
            THICKNESS_TEXT[self.thickness],
            BEZEL_TEXT[self.bezel],
            DISPERSION_TEXT[self.dispersion],
            IOR_TEXT[self.ior],
        )
    }

    /// 某一个参数加/减一档，自动夹在合法范围内。
    ///
    /// # 返回 `bool`（有没有真的动）
    ///
    /// 面板上的 `[−] [+] 在边界上是 no-op —— 而「点了没反应」必须与
    /// 「点了但被 clamp 回同一个值」区分得开，否则门禁
    /// `glass_params_are_not_hard_coded` 会把「clamp 生效」误判成「参数没接线」。
    pub fn nudge(&mut self, which: Param, delta: i32) -> bool {
        let slot = match which {
            Param::Thickness => &mut self.thickness,
            Param::Bezel => &mut self.bezel,
            Param::Dispersion => &mut self.dispersion,
            Param::Ior => &mut self.ior,
        };
        let max = match which {
            Param::Thickness => THICKNESS.len() - 1,
            Param::Bezel => BEZEL.len() - 1,
            Param::Dispersion => DISPERSION.len() - 1,
            Param::Ior => IOR.len() - 1,
        };
        // `as i64` 是必要的：`delta` 是 `i32`，而 `*slot` 可到 60，两者相加在
        // 理论上不溢出，但 `saturating_add` 的参数类型要求两边同宽。
        let next = (*slot as i64 + delta as i64).clamp(0, max as i64);
        let moved = next as usize != *slot;
        *slot = next as usize;
        moved
    }
}

/// 四个参数之一（用来索引 [`Glass::nudge`] 与 click 分派）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    Thickness,
    Bezel,
    Dispersion,
    Ior,
}

impl Param {
    /// 四个参数的稳定顺序 —— **门禁与样式表都按这个顺序**。
    ///
    /// 显式给 `const` 而不是靠枚举的声明序：后者一旦有人在中间插一个变体，
    /// 面板顺序就会静默改变，而画面上「还是那四个，只是换了位置」。
    pub const ALL: [Param; 4] = [
        Param::Thickness,
        Param::Bezel,
        Param::Dispersion,
        Param::Ior,
    ];

    /// 参数名（面板上的标签）。
    pub fn label(self) -> &'static str {
        match self {
            Param::Thickness => "Thickness",
            Param::Bezel => "Bezel",
            Param::Dispersion => "Dispersion",
            Param::Ior => "IOR",
        }
    }

    /// 该参数的**唯一 key 前缀**（`glass-<name>` 与 `glass-<name>:dec`）。
    pub fn slug(self) -> &'static str {
        match self {
            Param::Thickness => "thickness",
            Param::Bezel => "bezel",
            Param::Dispersion => "dispersion",
            Param::Ior => "ior",
        }
    }

    /// 该参数在 [`Glass::filter`] 四个字段里的**下标**（0..=3）。
    ///
    /// # 为什么门禁需要它
    ///
    /// 门禁要断的是「挪了 `Thickness`，树上变的**只有** thickness」——
    /// 而「四个字段全都变了」也会绿。一条只断「全都变了」的判据挡不住
    /// `nudge` 里少写一个字段、或者反过来错位写到别人字段上的形状。
    /// 有了这个下标，门禁才能逐字段断言「该变的变了，不该变的没变」。
    pub const fn index(self) -> usize {
        match self {
            Param::Thickness => 0,
            Param::Bezel => 1,
            Param::Dispersion => 2,
            Param::Ior => 3,
        }
    }
}

/// 厚度档位（像素）：0..=80，步长 4。
///
/// # 为什么上限是 80
///
/// 玻璃块本身 220px 宽（`pages.zstyle` 的 `.glass-pane`）。位移在倒角带紧贴
/// 边缘处可达 `厚度 × 数十`（`height` 的导数在 `t → 0` 处发散），所以再大的
/// 厚度只会被 `saturate` 夹到屏幕边缘 —— 那一段的差别已经看不出来了。
pub const THICKNESS: [f32; 21] = [
    0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 28.0, 32.0, 36.0, 40.0, 44.0, 48.0, 52.0, 56.0, 60.0,
    64.0, 68.0, 72.0, 76.0, 80.0,
];

/// 倒角带档位（像素）：2..=48，步长 2。
///
/// 下限 2 而不是 0：倒角带宽为 0 时 `t = -d / 0` 会除零，而着色器里的
/// `max(color.g, 1.0)` 只把它夹到 1 —— 那意味着「带外全部落到带内」，
/// 整块玻璃都在强折射区。上限 48 是因为 `min(bezel, radius, half_size)`
/// 会再被圆角半径（12px）压一次，所以超过 12 的档位在默认玻璃上**看不出差别**
/// —— 保留它们是为了让「调大了没变化」这件事本身可见。
pub const BEZEL: [f32; 24] = [
    2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 30.0, 32.0,
    34.0, 36.0, 38.0, 40.0, 42.0, 44.0, 46.0, 48.0,
];

/// 色散档位：0.00..=1.00，步长 0.05。
///
/// 上限 1.0 不是「最大色散」，而是「R / B 相对 G 的偏移比例 ±100%」——
/// 再大就只是把 R 拉得更远，不会更像玻璃。
pub const DISPERSION: [f32; 21] = [
    0.0, 0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75,
    0.80, 0.85, 0.90, 0.95, 1.0,
];

/// 折射率档位：1.00..=2.00，步长 0.05。
///
/// # 1.00 与 2.00 这两端各代表什么
///
/// - `1.00` = **没有折射**。着色器里 `asin(sin(Θs) / 1.0) = Θs`，位移
///   恰好为 0 —— 于是玻璃变成一块纯透镜板，什么都不弯。这条档位是**诊断
///   工具**：`IOR = 1` 时画面若仍有位移，说明公式被改坏了。
/// - `2.00` 超出任何真实玻璃（钻石 2.42、水 1.33、冕牌玻璃 1.45~1.52）。
///   保留它是为了让「IOR 越大越硬」这条趋势能被推到尽头看。
///
/// 1.50 正好落在第 10 档 —— 那就是默认值。
pub const IOR: [f32; 21] = [
    1.00, 1.05, 1.10, 1.15, 1.20, 1.25, 1.30, 1.35, 1.40, 1.45, 1.50, 1.55, 1.60, 1.65, 1.70, 1.75,
    1.80, 1.85, 1.90, 1.95, 2.00,
];

/// 各档的显示文本。
///
/// ⚠ 三张表与数值表**逐项对应**，长度必须相等 —— 门禁
/// `glass_param_tables_line_up` 钉住这一点。手写而不是 `format!` 的理由
/// 见 [`Glass`]。
pub const THICKNESS_TEXT: [&str; 21] = [
    "0", "4", "8", "12", "16", "20", "24", "28", "32", "36", "40", "44", "48", "52", "56", "60",
    "64", "68", "72", "76", "80",
];

/// 见 [`THICKNESS_TEXT`]。
pub const BEZEL_TEXT: [&str; 24] = [
    "2", "4", "6", "8", "10", "12", "14", "16", "18", "20", "22", "24", "26", "28", "30", "32",
    "34", "36", "38", "40", "42", "44", "46", "48",
];

/// 见 [`THICKNESS_TEXT`]。色散固定两位小数 —— 步长 0.05，一位小数会让
/// 「3」与「4」看起来一样。
pub const DISPERSION_TEXT: [&str; 21] = [
    "0.00", "0.05", "0.10", "0.15", "0.20", "0.25", "0.30", "0.35", "0.40", "0.45", "0.50", "0.55",
    "0.60", "0.65", "0.70", "0.75", "0.80", "0.85", "0.90", "0.95", "1.00",
];

/// 见 [`THICKNESS_TEXT`]。IOR 固定两位小数，且 1.50 / 1.00 这两个值对判读
/// 很关键（前者是普通玻璃，后者是「不折射」的诊断档）。
pub const IOR_TEXT: [&str; 21] = [
    "1.00", "1.05", "1.10", "1.15", "1.20", "1.25", "1.30", "1.35", "1.40", "1.45", "1.50", "1.55",
    "1.60", "1.65", "1.70", "1.75", "1.80", "1.85", "1.90", "1.95", "2.00",
];
