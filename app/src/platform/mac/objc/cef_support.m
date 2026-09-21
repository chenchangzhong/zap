// CEF(Chromium)后端在 macOS 上需要的两个宿主原语。
//
// 仅在 app 的 `cef_webview` feature 下编译(见 app/build.rs);默认构建不包含,
// 因此对现有行为零影响。归属:CEF 是 app 层的可选后端,故这些适配放在 app 侧,
// 不进 warpui。
//
// 1) 周期定时器——CEF 用 external_message_pump 时必须由宿主在**主线程稳定周期**
//    调用 do_message_loop_work();zap 自己的事件循环空闲时不产生帧,故需要独立
//    的 run loop 定时器(specs/cef-webview-minimal/TECH.md 集成要求 #4)。
// 2) NSApplication 协议桥——CEF 要求 NSApplication 实现 CefAppProtocol
//    (isHandlingSendEvent / setHandlingSendEvent:),zap 没有 NSApplication 子类,
//    故用 category 补方法 + 交换 sendEvent: 在其前后夹住 handling 标志
//    (集成要求 #3)。

#import <AppKit/AppKit.h>
#import <objc/runtime.h>

#pragma mark - 周期定时器

@interface WarpCefTimerTarget : NSObject
@property (nonatomic, assign) void (*callback)(void);
- (void)tick:(NSTimer *)timer;
@end

@implementation WarpCefTimerTarget
- (void)tick:(NSTimer *)timer {
    if (self.callback != NULL) {
        self.callback();
    }
}
@end

static WarpCefTimerTarget *gWarpCefTimerTarget = nil;
static NSTimer *gWarpCefTimer = nil;

/// 在**主线程** run loop 上启动(或重启)一个周期定时器。
/// 非 ARC 文件,故显式 retain/release(与 warpui 既有 .m 风格一致)。
void warp_cef_start_periodic_main_timer(double interval, void (*callback)(void)) {
    if (gWarpCefTimer != nil) {
        [gWarpCefTimer invalidate];
        [gWarpCefTimer release];
        gWarpCefTimer = nil;
    }
    if (gWarpCefTimerTarget != nil) {
        [gWarpCefTimerTarget release];
        gWarpCefTimerTarget = nil;
    }
    gWarpCefTimerTarget = [[WarpCefTimerTarget alloc] init];
    gWarpCefTimerTarget.callback = callback;
    gWarpCefTimer = [[NSTimer scheduledTimerWithTimeInterval:interval
                                                     target:gWarpCefTimerTarget
                                                   selector:@selector(tick:)
                                                   userInfo:nil
                                                    repeats:YES] retain];
    // 加入 common modes:滚动/拖拽等 tracking 期间也要继续推进 CEF。
    [[NSRunLoop mainRunLoop] addTimer:gWarpCefTimer forMode:NSRunLoopCommonModes];
}

#pragma mark - NSApplication 协议桥

static const void *kWarpCefHandlingSendEventKey = &kWarpCefHandlingSendEventKey;
static IMP gWarpCefOriginalSendEvent = NULL;

@interface NSApplication (WarpCefSupport)
- (BOOL)isHandlingSendEvent;
- (void)setHandlingSendEvent:(BOOL)value;
@end

@implementation NSApplication (WarpCefSupport)
- (BOOL)isHandlingSendEvent {
    NSNumber *value = objc_getAssociatedObject(self, kWarpCefHandlingSendEventKey);
    return value != nil && value.boolValue;
}

- (void)setHandlingSendEvent:(BOOL)value {
    objc_setAssociatedObject(self, kWarpCefHandlingSendEventKey, @(value),
                             OBJC_ASSOCIATION_RETAIN_NONATOMIC);
}
@end

/// 交换后的 sendEvent::在分发前后夹住 handling 标志(对齐 CEF 示例的契约)。
static void warp_cef_send_event(id self, SEL _cmd, NSEvent *event) {
    BOOL wasHandling = [self isHandlingSendEvent];
    if (!wasHandling) {
        [self setHandlingSendEvent:YES];
    }
    ((void (*)(id, SEL, NSEvent *))gWarpCefOriginalSendEvent)(self, _cmd, event);
    if (!wasHandling) {
        [self setHandlingSendEvent:NO];
    }
}

/// 安装 NSApplication 协议桥(幂等)。必须在创建任何 CEF 浏览器之前、主线程调用。
void warp_cef_install_app_protocol_support(void) {
    static BOOL installed = NO;
    if (installed) {
        return;
    }
    // 评审 F16 曾建议补 `class_addProtocol(NSApplication, @protocol(CefAppProtocol))`:
    // 未采纳 —— `CefAppProtocol` 只在 CEF 的 C++ 头(cef_application_mac.h)里声明,本文件是
    // 纯 ObjC(.m)无法引用;要补必须改成 .mm 并引入 CEF C++ 头(构建面变大)。当前 CEF
    // 按"是否响应方法"判定,实测(见 TRANSPARENCY/RUNTIME 证据)该桥有效,故保持零风险现状。

    Method sendEvent = class_getInstanceMethod([NSApplication class], @selector(sendEvent:));
    if (sendEvent != NULL) {
        gWarpCefOriginalSendEvent = method_getImplementation(sendEvent);
        method_setImplementation(sendEvent, (IMP)warp_cef_send_event);
    }
    installed = YES;
}
