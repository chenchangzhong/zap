// CEF 子视图与 zap 式挖洞/分层窗口的共存探针
// (specs/cef-webview-minimal/TECH.md 阶段 1 第一生死项)
//
// 复刻 zap 的真实拓扑(crates/warpui/src/platform/mac/objc/host_view.m 字段注释):
//   ProbeHostView(contentView, 模拟 WarpHostView)
//     ├─ ProbeContainerView(模拟 WebViewContainerView:透明、承载嵌入视图)
//     │    ├─ ProbeBackgroundView(模拟 MetalBackgroundView:铺满、最底层)
//     │    └─ [CEF 子视图](Rust 侧经 WindowInfo::set_as_child 挂入,坐标=洞)
//     └─ ProbeOverlayView(模拟 MetalRenderView:洞外绘制 UI、洞内透明且 hitTest 穿透)
//
// 事件路由:ProbeWindow 复刻 WarpWindow::sendEvent: 的 macOS 27 分支
// (crates/warpui/src/platform/mac/objc/window.m:460-556),含 WKWebView 类名特判;
// gZapRouting=0 时退化为 [super sendEvent:](= 平台默认),用于 A/B 对照:
//   A 平台默认 → CEF 应收到 mousedown+mouseup+click
//   B zap 路由 → 命中目标非 WKWebView ⇒ mouseUp 被改投 contentView,CEF 只收到 mousedown
// 说明:zap 版本还含窗口按钮/缩放边缘处理,与本探针要回答的问题无关,此处不复刻。

#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdio.h>
#include <stdlib.h>

static NSWindow *gWindow = nil;
static NSView *gContainer = nil;
static NSRect gHole = {{100, 100}, {600, 400}};
static int gRouting = 0;  // 0=平台默认 1=旧 zap 特判 2=泛化后的嵌入视图判定
static NSView *gLastMouseDownTarget = nil;  // 仅用于 dump(ivar 在方法外不可见)

void probe_dump_subviews(void);

#pragma mark - 背景层(模拟 MetalBackgroundView)

@interface ProbeBackgroundView : NSView
@end

@implementation ProbeBackgroundView
- (BOOL)isOpaque {
    return YES;
}
- (void)drawRect:(NSRect)dirtyRect {
    [[NSColor colorWithCalibratedRed:0.07 green:0.09 blue:0.12 alpha:1.0] setFill];
    NSRectFill(self.bounds);
}
@end

#pragma mark - 覆盖层(模拟 MetalRenderView:挖洞 + 命中穿透)

@interface ProbeOverlayView : NSView
@end

@implementation ProbeOverlayView
- (BOOL)isOpaque {
    return NO;
}
- (void)drawRect:(NSRect)dirtyRect {
    // 洞外:模拟"自绘 UI"(网格便于截图辨认)
    [[NSColor colorWithCalibratedRed:0.13 green:0.18 blue:0.26 alpha:1.0] setFill];
    NSRectFill(self.bounds);
    [[NSColor colorWithCalibratedWhite:1.0 alpha:0.06] setStroke];
    NSBezierPath *grid = [NSBezierPath bezierPath];
    [grid setLineWidth:1.0];
    for (CGFloat x = 0; x < self.bounds.size.width; x += 40) {
        [grid moveToPoint:NSMakePoint(x, 0)];
        [grid lineToPoint:NSMakePoint(x, self.bounds.size.height)];
    }
    for (CGFloat y = 0; y < self.bounds.size.height; y += 40) {
        [grid moveToPoint:NSMakePoint(0, y)];
        [grid lineToPoint:NSMakePoint(self.bounds.size.width, y)];
    }
    [grid stroke];
    // 洞:清除像素,露出下层 CEF 视图(对齐 Metal 渲染把背景 rect 拆到洞外)
    NSRectFillUsingOperation(gHole, NSCompositingOperationClear);
    [[NSColor colorWithCalibratedRed:0.35 green:0.9 blue:0.55 alpha:1.0] setStroke];
    NSBezierPath *border = [NSBezierPath bezierPathWithRect:NSInsetRect(gHole, 1.0, 1.0)];
    [border setLineWidth:2.0];
    [border stroke];
}
- (NSView *)hitTest:(NSPoint)point {
    // 洞内不拦事件,交给下层(CEF 子视图)
    if (NSPointInRect(point, gHole)) {
        return nil;
    }
    return [super hitTest:point];
}
@end

#pragma mark - 宿主 contentView

@interface ProbeHostView : NSView
@end

@implementation ProbeHostView
- (BOOL)isOpaque {
    return NO;
}
@end

#pragma mark - 窗口(复刻 WarpWindow::sendEvent: 的 macOS 27 分支)

@interface ProbeWindow : NSWindow
@end

@implementation ProbeWindow {
    NSView *_leftMouseDownTarget;
    BOOL _leftMouseDownStartedInNativeWindowChrome;
}

/// 命中目标是否属于嵌入平台视图(= WebViewContainerView 的后代)。
/// 这是"泛化为嵌入视图判定"(集成要求 #1)的参考实现。
static BOOL probe_is_embedded_view(NSView *view) {
    for (NSView *v = view; v; v = v.superview) {
        if (v == gContainer) {
            return YES;
        }
    }
    return NO;
}

- (void)sendEvent:(NSEvent *)event {
    if (gRouting == 0) {
        [super sendEvent:event];
        return;
    }
    const BOOL treatAsEmbedded =
        (gRouting == 2) ? probe_is_embedded_view(_leftMouseDownTarget)
                        : [_leftMouseDownTarget isKindOfClass:NSClassFromString(@"WKWebView")];
    switch (event.type) {
        case NSEventTypeLeftMouseDown: {
            _leftMouseDownStartedInNativeWindowChrome = NO;
            NSPoint contentPoint = [self.contentView convertPoint:event.locationInWindow fromView:nil];
            _leftMouseDownTarget = [self.contentView hitTest:contentPoint];
            gLastMouseDownTarget = _leftMouseDownTarget;
            [super sendEvent:event];
            break;
        }
        case NSEventTypeLeftMouseUp:
            if (@available(macOS 27, *)) {
                printf("[probe] mouseUp branch=%s target=%s\n",
                       (_leftMouseDownStartedInNativeWindowChrome || treatAsEmbedded)
                           ? "super(交给命中视图)" : "contentView(改投!)",
                       gLastMouseDownTarget ? class_getName([gLastMouseDownTarget class]) : "(nil)");
                fflush(stdout);
                if (_leftMouseDownStartedInNativeWindowChrome || treatAsEmbedded) {
                    [super sendEvent:event];
                } else {
                    [self.contentView mouseUp:event];
                }
            } else {
                [self.contentView mouseUp:event];
            }
            // 事件序列结束:复位跟踪状态(与 zap 一致,避免悬垂引用)。
            _leftMouseDownTarget = nil;
            _leftMouseDownStartedInNativeWindowChrome = NO;
            break;
        case NSEventTypeLeftMouseDragged:
            if (@available(macOS 27, *)) {
                if (_leftMouseDownStartedInNativeWindowChrome || treatAsEmbedded) {
                    [super sendEvent:event];
                } else {
                    [self.contentView mouseDragged:event];
                }
            } else {
                [self.contentView mouseDragged:event];
            }
            break;
        default:
            [super sendEvent:event];
            break;
    }
}
@end

#pragma mark - Rust 侧 FFI

/// 建窗并展示。**必须在 CEF 的 setup_simple_application 之后调用**
/// (cef-rs mac 模块断言 NSApp 在此之前未被触碰)。
/// 让探针把日志写进文件:经 LaunchServices(`open`)启动时无 stdout 可读。
void probe_redirect_stdout(const char *path) {
    if (path && path[0]) {
        freopen(path, "w", stdout);
    }
}

void probe_create_window(double hx, double hy, double hw, double hh) {
    gHole = NSMakeRect(hx, hy, hw, hh);
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSRect frame = NSMakeRect(160, 160, 960, 640);
    gWindow = [[ProbeWindow alloc] initWithContentRect:frame
                                             styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                                        NSWindowStyleMaskResizable)
                                               backing:NSBackingStoreBuffered
                                                 defer:NO];
    [gWindow setTitle:@"cef hole probe (阶段1 生死项)"];

    NSView *content = [[ProbeHostView alloc] initWithFrame:frame];
    content.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [gWindow setContentView:content];

    gContainer = [[NSView alloc] initWithFrame:content.bounds];
    gContainer.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    gContainer.wantsLayer = YES;
    gContainer.layer.backgroundColor = NSColor.clearColor.CGColor;
    [content addSubview:gContainer];

    ProbeBackgroundView *bg = [[ProbeBackgroundView alloc] initWithFrame:gContainer.bounds];
    bg.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [gContainer addSubview:bg];

    ProbeOverlayView *overlay = [[ProbeOverlayView alloc] initWithFrame:content.bounds];
    overlay.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [content addSubview:overlay];

    [gWindow makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    printf("[probe] window created hole=(%.0f,%.0f,%.0f,%.0f) windowNumber=%ld\n", hx, hy, hw, hh,
           (long)gWindow.windowNumber);
    printf("[probe] window frame=%s content=%s\n",
           NSStringFromRect(gWindow.frame).UTF8String,
           NSStringFromRect(gWindow.contentView.frame).UTF8String);
    fflush(stdout);
}

/// CEF 子视图的父视图(= 洞的容器)。
void *probe_container_ptr(void) {
    return (__bridge void *)gContainer;
}

void probe_set_routing(int mode) {
    gRouting = mode;
    printf("[probe] routing=%s\n", mode == 0 ? "plain(平台默认)" : (mode == 1 ? "zap(旧 WKWebView 特判)" : "embedded(泛化判定)"));
    fflush(stdout);
}

long probe_window_number(void) {
    return (long)gWindow.windowNumber;
}

static void probe_send_mouse(NSEventType type) {
    NSPoint inWindow = [gWindow.contentView convertPoint:NSMakePoint(NSMidX(gHole), NSMidY(gHole))
                                                  toView:nil];
    NSEvent *event = [NSEvent mouseEventWithType:type
                                        location:inWindow
                                   modifierFlags:0
                                       timestamp:NSProcessInfo.processInfo.systemUptime
                                    windowNumber:gWindow.windowNumber
                                         context:nil
                                     eventNumber:(NSInteger)(type == NSEventTypeLeftMouseDown ? 1 : 2)
                                      clickCount:1
                                        pressure:1.0];
    // 直接投给 window.sendEvent:——测的正是 WarpWindow::sendEvent 的分流分支,
    // 且不受"应用是否被 LaunchServices 激活/窗口是否 key"影响(该前置检查由
    // NSApplication 做;经 NSApp postEvent 的路径在未激活时会丢失事件)。
    printf("[probe] sendMouse type=%ld isKeyWindow=%d appActive=%d\n", (long)type,
           (int)gWindow.isKeyWindow, (int)NSApp.isActive);
    fflush(stdout);
    [gWindow sendEvent:event];
}

/// 打印容器子视图的 frame 与命中测试结果(诊断 CEF 子视图实际落点 vs 洞)。
void probe_dump_subviews(void) {
    printf("[probe] container frame=%s bounds=%s\n",
           NSStringFromRect(gContainer.frame).UTF8String, NSStringFromRect(gContainer.bounds).UTF8String);
    for (NSView *v in gContainer.subviews) {
        printf("[probe]   subview %s frame=%s hidden=%d alpha=%.2f\n", class_getName([v class]),
               NSStringFromRect(v.frame).UTF8String, (int)v.hidden, v.alphaValue);
    }
    NSPoint holeCenter = NSMakePoint(NSMidX(gHole), NSMidY(gHole));
    NSView *hit = [gWindow.contentView hitTest:holeCenter];
    printf("[probe] hitTest(hole center %.0f,%.0f) -> %s\n", holeCenter.x, holeCenter.y,
           hit ? class_getName([hit class]) : "(nil)");
    NSPoint outside = NSMakePoint(20, 20);
    NSView *hit2 = [gWindow.contentView hitTest:outside];
    printf("[probe] hitTest(outside %.0f,%.0f) -> %s\n", outside.x, outside.y,
           hit2 ? class_getName([hit2 class]) : "(nil)");
    // 供像素采样换算:窗口/洞中心在屏幕坐标(底部原点)+ 屏幕与缩放比。
    {
        NSScreen *screen = gWindow.screen;
        NSRect sf = screen ? screen.frame : NSZeroRect;
        CGFloat scale = screen ? screen.backingScaleFactor : 2.0;
        NSPoint inWin = [gWindow.contentView convertPoint:NSMakePoint(NSMidX(gHole), NSMidY(gHole))
                                                    toView:nil];
        NSPoint inScreen = [gWindow convertPointToScreen:inWin];
        printf("[probe] shotmap screen=%.0fx%.0f scale=%.0f holeCenterScreen=(%.0f,%.0f)\n",
               sf.size.width, sf.size.height, scale, inScreen.x, inScreen.y);
    }
    printf("[probe] lastLeftMouseDownTarget=%s\n",
           gLastMouseDownTarget ? class_getName([gLastMouseDownTarget class]) : "(nil)");
    NSView *frv = (NSView *)[gWindow firstResponder];
    printf("[probe] isKeyWindow=%d appActive=%d firstResponder=%s\n", (int)gWindow.isKeyWindow,
           (int)NSApp.isActive, frv ? class_getName([frv class]) : "(nil)");
    fflush(stdout);
}

/// 在洞中心合成一次左键点击(进程内合成,不走辅助功能权限)。
/// 页面事件只在窗口为 key 时才会被 Chromium 转发,故先反复尝试激活:
/// 一旦 isKeyWindow 为真立刻点击;超时则照点(分支日志仍是有效证据,但页面级证据不计入)。
static void probe_click_attempt(int attempt) {
    if (gWindow.isKeyWindow || attempt >= 12) {
        printf("[probe] CLICK_PROCEEDING (key=%d active=%d attempt=%d;分支日志为主证据)\n",
               (int)gWindow.isKeyWindow, (int)NSApp.isActive, attempt);
        fflush(stdout);
        [gWindow makeKeyAndOrderFront:nil];
        probe_dump_subviews();
        probe_send_mouse(NSEventTypeMouseMoved);
        probe_send_mouse(NSEventTypeLeftMouseDown);
        probe_send_mouse(NSEventTypeLeftMouseUp);
        printf("[probe] click sent\n");
        fflush(stdout);
        return;
    }
    [NSApp activateIgnoringOtherApps:YES];
    [[NSRunningApplication currentApplication]
        activateWithOptions:NSApplicationActivateIgnoringOtherApps];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.2 * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                       probe_click_attempt(attempt + 1);
                   });
}

void probe_schedule_click(double delay) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(delay * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                       printf("[probe] synthesizing click at hole center\n");
                       fflush(stdout);
                       probe_click_attempt(0);
                   });
}

/// 放大窗口(+dw/+dh),用于测 CEF 子视图是否跟随 host 布局变化。
void probe_schedule_resize(double delay, double dw, double dh) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(delay * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                       NSRect f = gWindow.frame;
                       f.size.width += dw;
                       f.size.height += dh;
                       [gWindow setFrame:f display:YES];
                       printf("[probe] window resized by (+%.0f,+%.0f) -> %s\n", dw, dh,
                              NSStringFromRect(f).UTF8String);
                       fflush(stdout);
                   });
}

void probe_schedule_exit(double delay) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(delay * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                       printf("[probe] exit\n");
                       fflush(stdout);
                       exit(0);
                   });
}
