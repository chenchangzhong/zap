#!/bin/bash
# 阶段 1 生死项探针的一键复现:启动 → 全屏截图 → 像素采样 → 收集分支与事件证据。
#
# 用法: probes/run_hole_probe.sh <plain|zap|embedded> <tag> [额外探针参数...]
#   例: probes/run_hole_probe.sh embedded C_embedded
#       probes/run_hole_probe.sh plain resize --resize-test
#
# 环境变量:
#   EVIDENCE_DIR  证据输出目录(默认 specs/cef-webview-minimal/evidence/phase1)
#   MAX_ATTEMPTS  未取到 shotmap 时的重试次数(默认 4;应用激活/窗口前置可能失败)
#
# 产物:
#   <tag>_probe.log    探针日志(routing / shotmap / hitTest / mouseUp 分支 / 自校验)
#   <tag>_server.log   页面经回环通道上报的事件(loaded/mousedown/mouseup/click)
#   <tag>_shot.png     全屏截图(定向 -l 截图在缺"屏幕录制"权限时不可用)
#   <tag>_pixels.txt   像素采样:洞内应为页面渐变,洞外应为覆盖层底色
set -uo pipefail

ROUTING="${1:?usage: run_hole_probe.sh <plain|zap|embedded> <tag> [extra args]}"
TAG="${2:?usage: run_hole_probe.sh <plain|zap|embedded> <tag> [extra args]}"
shift 2 || true

HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/.." && pwd)"
EVIDENCE_DIR="${EVIDENCE_DIR:-$CRATE/../../specs/cef-webview-minimal/evidence/phase1}"
PORT=9911
HOLE_X=100; HOLE_Y=100; HOLE_W=600; HOLE_H=400
MAX_ATTEMPTS="${MAX_ATTEMPTS:-4}"
APP="$CRATE/target/hole/hole-probe.app"

# make-bundle 重建后签名失效,LaunchServices 会以 code=5 拒绝启动;每次运行前重签。
codesign --force --deep -s - "$APP" >/dev/null 2>&1
mkdir -p "$EVIDENCE_DIR"
[ -x "$HERE/sample_pixel" ] || swiftc -O "$HERE/sample_pixel.swift" -o "$HERE/sample_pixel"

# 回环接收端(端口 TIME_WAIT 时也能立即重绑)
lsof -ti:$PORT 2>/dev/null | xargs kill -9 2>/dev/null
sleep 0.3
python3 "$HERE/loopback_receiver.py" > "$EVIDENCE_DIR/${TAG}_server.log" 2>&1 &
RECEIVER_PID=$!

PLOG="$EVIDENCE_DIR/${TAG}_probe.log"
rm -f "$PLOG" "$EVIDENCE_DIR/${TAG}_shot.png" "$EVIDENCE_DIR/${TAG}_pixels.txt"

for ATTEMPT in $(seq 1 "$MAX_ATTEMPTS"); do
  open -n "$APP" --args \
    --url="file://${PAGE:-$HERE/hole_page.html}" \
    --routing="$ROUTING" \
    --hole="$HOLE_X,$HOLE_Y,$HOLE_W,$HOLE_H" \
    --click-after=5 --exit-after=10 \
    --log="$PLOG" "$@"

  for _ in $(seq 1 40); do
    grep -q "WINDOW_NUMBER=" "$PLOG" 2>/dev/null && break
    sleep 0.25
  done
  sleep 2
  screencapture -x "$EVIDENCE_DIR/${TAG}_shot.png" 2>/dev/null

  for _ in $(seq 1 60); do
    pgrep -f "hole-probe.app/Contents/MacOS/hole-probe" >/dev/null || break
    sleep 0.5
  done
  grep -q "shotmap " "$PLOG" 2>/dev/null && break
  echo "  (attempt $ATTEMPT 未取到 shotmap,重试)"
  sleep 1
done
kill "$RECEIVER_PID" 2>/dev/null

# ---- 像素采样:洞内 vs 洞外(解析与换算交给 compute_sample_points.py) ----
SHOT="$EVIDENCE_DIR/${TAG}_shot.png"
if [ -f "$SHOT" ]; then
  POINTS=$(python3 "$HERE/compute_sample_points.py" "$PLOG" 2>/dev/null)
  if [ -n "$POINTS" ] && [ -x "$HERE/sample_pixel" ]; then
    IN_PT=$(echo "$POINTS" | cut -d' ' -f1)
    OUT_PT=$(echo "$POINTS" | cut -d' ' -f2)
    {
      echo "# routing=$ROUTING tag=$TAG"
      echo "# 采样点: IN_HOLE=($IN_PT) 期望=页面渐变(偏绿); OUTSIDE=($OUT_PT) 期望=覆盖层底色(#212E42 附近,偏蓝)"
      "$HERE/sample_pixel" "$SHOT" "$IN_PT" "$OUT_PT"
    } > "$EVIDENCE_DIR/${TAG}_pixels.txt" 2>&1
  fi
fi

echo "--- $TAG (routing=$ROUTING) ---"
grep -E "routing=|shotmap|lastLeftMouseDownTarget|mouseUp branch=|CLICK_PROCEEDING" "$PLOG" 2>/dev/null
echo "页面事件 $(grep -c 'zap-ipc-received' "$EVIDENCE_DIR/${TAG}_server.log" 2>/dev/null) 条:" \
     "$(grep 'zap-ipc-received' "$EVIDENCE_DIR/${TAG}_server.log" 2>/dev/null | sed 's/.*] //' | tr '\n' ' ')"
[ -f "$EVIDENCE_DIR/${TAG}_pixels.txt" ] && cat "$EVIDENCE_DIR/${TAG}_pixels.txt"
