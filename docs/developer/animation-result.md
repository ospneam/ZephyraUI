## 动画（`docs/animation.md`）

> 出处：<https://learn.microsoft.com/en-us/windows/apps/design/motion/>
> 「动画属性」表格 —— 缓动与时长的**唯一**来源。改任何常量先回查该页。

### 官方表（逐字抄录， Stan 不许自己拍）

| 用途 | 定义 | Easing | Duration | Used for |
|---|---|---|---|---|
| Direct entry | Fast in | cubic-bezier(0, 0, 0, 1) | 167 / 250 / **333** ms | Position, scale, rotate |
| Existing element | Point-to-point | cubic-bezier(0.55, 0.55, 0, 1) | 167 / 250 / **333** ms | Position, scale, rotate |
| Direct exit | Fast out | cubic-bezier(0, 0, 0, 1) | **167** ms | Position, scale, rotate (**always combined with fade**) |
| Gentle exit | Soft out | cubic-bezier(1, 0, 1, 1) | **167** ms | Position, scale |
| Minimum | Fade in + out | **Linear** | **83** ms | **Opacity** |
| Powerful entry | Back-in (3 keyframes) | (0.85,0,0,1) / (0.85,0,0.75,1) / (0.85,0,0,1) | 167 / 167 / 333 ms | Position, scale |

### 三条不能自己改的东西

1. **83 ms + linear is only for opacity** —— not a general short duration.
2. Point-to-point is `(0.55,0.55,0,1)`, **not** the same as direct entry's `(0,0,0,1)`.
3. Direct exit **must** combine with a fade —— position/scale may not disappear on
   their own. (This one is the one most likely to be dropped when porting.)

### Where this repo puts animations

`zephyra_ui::anim` — easing/clock/progress, **pure functions only**. It holds no
state and touches no appearance, so

- the **file-level gate** `the_whole_tree_writes_no_appearance_in_source` cannot
  trip on it: progress (0..1) is computed here, but the value it maps to stays in
  the stylesheet (see §3 of `docs/animation.md`);
- easing correctness is asserted point by point in CI instead of by eye.

### The piece that is deliberately NOT built yet

The official table's properties are **opacity** and **position**; this repo's core
supports neither for animation:

| Needed | Today | What is missing |
|---|---|---|
| Opacity | `Quad.fill` is `Color` (has alpha), but containers have no aggregate alpha | zstyle has no opacity property, so even a static translucent container cannot be written |
| Position | layout computes the rect; render reads it directly | no "add an interpolated offset at draw time" layer |

Faking it would mean mapping progress onto **colour/radius**, which reads as a
joke next to the real thing. So this round lands only the floor — easing, clock,
quantisation (all unit-tested) — and books the capability gap.

### Two things the unit tests caught in *my own* implementation

1. **Newton iteration did not clamp `s` to [0,1]**. For `x_of(s)=s³` the
   derivative `3s²` tends to 0, so one step threw `s` to 16.8, `y_of` overflowed
   and got clamped to 0.9989 — the curve appeared to *go backwards* at t=0.03.
   That failure mode looks nothing like "wrong constant".
2. **I asserted "point-to-point should sit near 0.5"**. Measured: 0.92. All three
   official curves have their y control points high, so all three are
   fast-then-slow. The doc gives **control point literals**; the shape follows the
   literals, not the name. I had guessed from the name.

### Follow-up (booked in `REWRITE.md`)

1. core: container opacity + a draw-time interpolated offset;
2. then `.enter-<n>` step classes in the stylesheet, driven by the clock.
