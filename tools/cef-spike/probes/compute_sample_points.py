#!/usr/bin/env python3
"""从探针日志解析坐标映射,算出洞内/洞外两个采样点(图像像素,顶部原点)。

用法: compute_sample_points.py <probe.log>   →  stdout: "<x_in>,<y_in> <x_out>,<y_out>"
解析失败时退出码 2(调用方据此判定该轮证据不可用)。
"""
import re
import sys

log = open(sys.argv[1], encoding="utf-8", errors="replace").read()

m = re.search(
    r"shotmap screen=(?P<sw>[\d.]+)x(?P<sh>[\d.]+) scale=(?P<scale>[\d.]+) "
    r"holeCenterScreen=\(\s*(?P<cx>[\d.-]+)\s*,\s*(?P<cy>[\d.-]+)\s*\)",
    log,
)
w = re.search(r"window frame=\{\{(?P<x>[\d.-]+), (?P<y>[\d.-]+)\}, \{[^}]*\}", log)
c = re.search(r"content=\{\{[^}]*\}, \{(?P<w>[\d.-]+), (?P<h>[\d.-]+)\}\}", log)
if not (m and w and c):
    sys.exit(2)

scale = float(m["scale"])
screen_h = float(m["sh"])
x_in = round(float(m["cx"]) * scale)
y_in = round((screen_h - float(m["cy"])) * scale)
# 洞外参考点:窗口内容顶部内侧 20pt(洞占 (100,100)-(700,500),此处必在洞外)
y_top_content = float(w["y"]) + float(c["h"]) - 20
x_out = round((float(w["x"]) + 30) * scale)
y_out = round((screen_h - y_top_content) * scale)
print(f"{x_in},{y_in} {x_out},{y_out}")
