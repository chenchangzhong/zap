// OSR 探针(T1)的宿主视图:把 CEF 经 on_accelerated_paint 送来的 IOSurface 贴到
// CALayer.contents 上(零拷贝)。拓扑与 windowed 探针完全一致:本视图加进 probe 容器的
// "洞"位置,覆盖层仍在最上层且洞内透明 ⇒ 洞内看到的就是本视图合成后的像素,
// 于是 screencapture + sample_pixel 的采样口径与 windowed 探针可比。
//
// 关键契约(CEF 头文件):accelerated paint 的共享句柄"每帧可能不同、不可缓存、
// 回调返回即回收",所以每帧都要重新贴,并且必须放在 CATransaction 里禁用隐式动画。
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <IOSurface/IOSurface.h>
#include <stdint.h>

static NSView *gOsrView = nil;
static NSTimer *gBeginFrameTimer = nil;

/// 创建 OSR 宿主视图并加入容器(参数为容器坐标,与 CEF 的 windowed 子视图同口径)。
void *osr_host_view_new(void *container, double x, double y, double w, double h) {
    NSView *parent = (__bridge NSView *)container;
    NSRect frame = NSMakeRect(x, y, w, h);
    NSView *view = [[NSView alloc] initWithFrame:frame];
    view.wantsLayer = YES;
    view.layer.backgroundColor = NSColor.clearColor.CGColor;
    view.layer.opaque = NO;
    view.layer.contentsGravity = kCAGravityResize;
    CGFloat scale = parent.window ? parent.window.backingScaleFactor : 2.0;
    view.layer.contentsScale = scale;
    [parent addSubview:view];
    gOsrView = view;
    printf("[osr] host view created frame=%s scale=%.2f\n",
           NSStringFromRect(frame).UTF8String, (double)scale);
    fflush(stdout);
    return (__bridge void *)view;
}

/// 贴一帧:layer.contents = IOSurface。
void osr_host_view_set_surface(void *view, void *surface) {
    if (view == NULL || surface == NULL) {
        return;
    }
    NSView *v = (__bridge NSView *)view;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v.layer.contents = (__bridge id)surface;
    [CATransaction commit];
}

void osr_host_view_set_frame(void *view, double x, double y, double w, double h) {
    if (view == NULL) {
        return;
    }
    NSView *v = (__bridge NSView *)view;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v.frame = NSMakeRect(x, y, w, h);
    [CATransaction commit];
}

void osr_host_view_set_hidden(void *view, int hidden) {
    if (view == NULL) {
        return;
    }
    ((__bridge NSView *)view).hidden = hidden ? YES : NO;
}

void osr_host_view_remove(void *view) {
    if (view == NULL) {
        return;
    }
    [(__bridge NSView *)view removeFromSuperview];
}

double osr_host_view_scale(void *view) {
    NSView *v = (__bridge NSView *)view;
    if (v.window) {
        return (double)v.window.backingScaleFactor;
    }
    return 2.0;
}

/// 打印宿主视图的 layer 状态(证据用:确认 contents 已被贴上、是否非不透明)。
void osr_host_view_dump_layer(void *view) {
    NSView *v = (__bridge NSView *)view;
    printf("[osr] layer contents=%p opaque=%d contentsScale=%.2f bounds=%s\n",
           (__bridge void *)v.layer.contents, (int)v.layer.opaque,
           (double)v.layer.contentsScale,
           NSStringFromRect(v.layer.bounds).UTF8String);
    fflush(stdout);
}

/// 直接读回 layer.contents 指向的 IOSurface 像素(含 alpha)——不受窗口遮挡影响,
/// 是"CEF 到底画了什么、透明区是否真的 alpha=0"的最硬证据。
void osr_surface_dump(void *view) {
    if (view == NULL) {
        printf("[osr] surface: view is null\n");
        fflush(stdout);
        return;
    }
    NSView *v = (__bridge NSView *)view;
    id contents = v.layer.contents;
    if (contents == nil) {
        printf("[osr] surface: layer.contents is nil\n");
        fflush(stdout);
        return;
    }
    if (CFGetTypeID((__bridge CFTypeRef)contents) != IOSurfaceGetTypeID()) {
        printf("[osr] surface: contents is not IOSurface\n");
        fflush(stdout);
        return;
    }
    IOSurfaceRef surface = (__bridge IOSurfaceRef)contents;
    IOSurfaceLock(surface, kIOSurfaceLockReadOnly, NULL);
    size_t w = IOSurfaceGetWidth(surface);
    size_t h = IOSurfaceGetHeight(surface);
    size_t bpr = IOSurfaceGetBytesPerRow(surface);
    uint8_t *base = IOSurfaceGetBaseAddress(surface);
    OSType fmt = IOSurfaceGetPixelFormat(surface);
    printf("[osr] surface %zux%zu bpr=%zu fmt=0x%08X ('%c%c%c%c')\n", w, h, bpr, (unsigned)fmt,
           (char)(fmt >> 24), (char)(fmt >> 16), (char)(fmt >> 8), (char)fmt);
    struct {
        const char *name;
        size_t x;
        size_t y;
    } points[] = {
        {"center", w / 2, h / 2},
        {"quarter", w / 4, h / 4},
        {"corner", 4, 4},
        {"botright", w - 6, h - 6},
    };
    for (int i = 0; i < 4; i++) {
        uint8_t *p = base + points[i].y * bpr + points[i].x * 4;
        printf("[osr]   %-9s bytes=[%3u %3u %3u %3u]\n", points[i].name, p[0], p[1], p[2], p[3]);
    }
    size_t zero = 0;
    size_t total = 0;
    for (size_t y = 0; y < h; y += 4) {
        uint8_t *row = base + y * bpr;
        for (size_t x = 0; x < w; x += 4) {
            total++;
            if (row[x * 4 + 3] == 0) {
                zero++;
            }
        }
    }
    printf("[osr]   alpha==0: %zu/%zu = %.1f%%\n", zero, total,
           total ? 100.0 * (double)zero / (double)total : 0.0);
    IOSurfaceUnlock(surface, kIOSurfaceLockReadOnly, NULL);
    fflush(stdout);
}

/// 外部 begin-frame 驱动(可选):CEF 的 external_begin_frame_enabled 要求宿主按显示刷新
/// 节奏调用 send_external_begin_frame,否则不会绘制。默认不启用(见 osr-probe 的开关)。
void osr_start_begin_frame_timer(double interval, void (*callback)(void)) {
    if (gBeginFrameTimer != nil) {
        return;
    }
    gBeginFrameTimer = [NSTimer scheduledTimerWithTimeInterval:interval
                                                       repeats:YES
                                                         block:^(NSTimer *timer) {
                                                           (void)timer;
                                                           callback();
                                                         }];
    [[NSRunLoop mainRunLoop] addTimer:gBeginFrameTimer forMode:NSRunLoopCommonModes];
    printf("[osr] begin-frame timer started interval=%.4f\n", interval);
    fflush(stdout);
}

void osr_stop_begin_frame_timer(void) {
    [gBeginFrameTimer invalidate];
    gBeginFrameTimer = nil;
}
