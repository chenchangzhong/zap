# OSR Spike A:windowless + IOSurface → CALayer 真透明验证(通过)

> 目的:验证 `OSR-PLAN.md` T1 —— 在 zap 的分层拓扑下,CEF 的 windowless 渲染能否**真透明**
> (透明区透出下层),以及共享纹理链路是否可用。
> 结论:**通过**。并顺带修掉一个"缩放被乘两次"的实现陷阱。

## 1 环境与命令

```bash
cd tools/cef-spike
CEF_PATH="$HOME/.local/share/cef" cargo run --bin make-bundle -- osr-probe -o target/osr
codesign --force --deep -s - target/osr/osr-probe.app
PROBE_TRANSPARENT=1 target/osr/osr-probe.app/Contents/MacOS/osr-probe \
  --url="file://$PWD/probes/osr_mix_page.html" --routing=plain \
  --hole=100,100,600,400 --click-after=6 --exit-after=11 --log=/tmp/osr.log
grep -a "^\[osr\] surface\|^\[osr\]   " /tmp/osr.log
```

探针拓扑与 windowed 探针一致(复用 `probes/hole_probe.m`):底层 `ProbeBackgroundView`(#12171F)
→ 容器 → 覆盖层(洞内透明、洞外 #212E42)。唯一区别:网页像素不是 CEF 画进子视图,而是
`on_accelerated_paint` → IOSurface → 自建 NSView 的 `CALayer.contents`(`probes/osr_host.m`)。

测试页 `probes/osr_mix_page.html`:html/body 透明 + 中央不透明红块(#FF2D55)+ 右下角旋转动画
(持续产生 damage,避免只画首帧)。

## 2 证据(直接读回 IOSurface 像素,不受窗口遮挡影响)

```
[osr] view_rect=600x400 DIP (scale=2, 洞=600x400 点)
[osr] browser created windowless=1 shared_texture=1 external_begin_frame=0 bg=0x00000000 frame_rate=60 create_ret=1
[osr] on_after_created
[osr] accelerated paint #1 surface=0x… view=0x… dirty=1
[osr] layer contents=0x… opaque=0 contentsScale=2.00 bounds={{0, 0}, {600, 400}}
[osr] surface 1200x800 bpr=4864 fmt=0x42475241 ('BGRA')
[osr]   center    bytes=[ 85  45 255 255]     ← BGRA ⇒ #FF2D55 红块,alpha=255(内容确实渲染)
[osr]   quarter   bytes=[  0   0   0   0]     ← 完全透明
[osr]   corner    bytes=[  0   0   0   0]
[osr]   botright  bytes=[  0   0   0   0]
[osr]   alpha==0: 49590/60000 = 82.7%         ← 透明区是真的 alpha=0
```

对照:同页换成不透明渐变页(`hole_page.html`)时,洞内渲染出绿色渐变(屏幕采样 #0E5F4A/#166F55),
证明链路对不透明内容同样正确。

帧率:带 CSS 动画时 40 秒内记录 2460+ 次 `accelerated paint`(≈60fps),
说明 `windowless_frame_rate=60` + 内部绘制节奏可用(未启用 external begin-frame)。

## 3 结论

1. **windowless + `background_color` alpha=0 + 页面透明 ⇒ CEF 输出带 alpha 的帧**(实测 82.7% 像素 alpha=0),
   贴进 `CALayer.contents` 后下层正常透出 ⇒ **真透明可行**,windowed 的限制不适用于 OSR。
2. 共享纹理链路可用:`on_accelerated_paint` 的 `shared_texture_io_surface` 可直接作为
   `layer.contents`(零拷贝),且必须每帧重设(句柄会变),放在 `CATransaction` 里禁用隐式动画。
3. **实现陷阱(已在探针修正,主仓 T4 必须遵守)**:`view_rect` 与 `ScreenInfo.rect` 用 **DIP(逻辑点)**,
   由 CEF 乘 `device_scale_factor`;若返回像素又报 scale=2,会**重复缩放**(实测 surface 变成
   2400x1600 而非 1200x800,浪费显存并可能模糊)。
4. 探针骨架位置:`tools/cef-spike/src/bin/osr-probe.rs` + `probes/osr_host.m`
   (含 IOSurface 像素读回 `osr_surface_dump`,可作后续回归的硬证据)。

## 4 尚未验证(T2 起)

- IME(中文输入)链路 —— **否决级**,见 `OSR-PLAN.md` T2。
- `<select>` 等页面内弹层(探针目前只记录 `on_popup_show/on_popup_size`)。
- 输入转发(鼠标/键盘/焦点)与编辑命令 —— T5。
- `external_begin_frame` 宿主驱动(探针有开关 `PROBE_OSR_EXTERNAL_BEGIN_FRAME=1`,未实测)。
