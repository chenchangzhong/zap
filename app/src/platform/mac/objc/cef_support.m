// CEF(Chromium)后端在 macOS 上需要的宿主原语。
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
// 3) OSR(windowless)宿主视图——windowless 下 CEF 不创建原生视图,网页像素经
//    `on_accelerated_paint` 的 IOSurface 送到宿主,由我们贴进自建 NSView 的 CALayer
//    (specs/cef-webview-minimal/OSR-PLAN.md T4;手法与 T1 探针一致)。
//
// 本文件是**非 ARC**(build.rs 未加 -fobjc-arc),故所有对象生命周期显式配对。

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <IOSurface/IOSurface.h>
#import <objc/runtime.h>
#include <stdint.h>

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

#pragma mark - OSR(windowless)宿主视图

/// OSR 输入事件类型(与 Rust 侧 `WarpCefOsrInputKind` 一一对应)。
enum {
    WARP_CEF_OSR_EVENT_MOUSE_MOVE = 0,
    WARP_CEF_OSR_EVENT_MOUSE_CLICK = 1,
    WARP_CEF_OSR_EVENT_MOUSE_WHEEL = 2,
    WARP_CEF_OSR_EVENT_KEY = 3,
};

/// 标准编辑动作(与 Rust 侧 `WarpCefOsrEditCommand` 一一对应)。
enum {
    WARP_CEF_OSR_EDIT_COPY = 0,
    WARP_CEF_OSR_EDIT_CUT = 1,
    WARP_CEF_OSR_EDIT_PASTE = 2,
    WARP_CEF_OSR_EDIT_SELECT_ALL = 3,
    WARP_CEF_OSR_EDIT_UNDO = 4,
    WARP_CEF_OSR_EDIT_REDO = 5,
};

/// 传给 Rust 的输入事件。**字段顺序/类型必须与 Rust 侧 `WarpCefOsrInputEvent` 完全一致。**
///
/// 这里只做"原始采集":修饰键位映射、mac→Windows 键码表、CEF 事件结构体构造都在
/// Rust 侧完成(那里有 cef 绑定,且这些纯逻辑可以单测)。
typedef struct {
    int32_t type;
    /// 视图坐标:**左上原点、DIP(逻辑点)** —— CEF 鼠标事件的口径。
    double x;
    double y;
    /// NSEventModifierFlags 原始位。
    uint32_t modifier_flags;
    int32_t click_count;
    /// 0=左 1=右 2=中;-1 = 没有按键被按住(纯移动,用于区分 hover 与拖拽)
    int32_t button;
    int32_t mouse_up;
    int32_t mouse_leave;
    /// 滚轮增量(CGEvent point delta,与 cefclient 同源)。
    int32_t delta_x;
    int32_t delta_y;
    /// 0=keyDown 1=keyUp 2=flagsChanged
    int32_t key_type;
    /// macOS 虚拟键码。
    uint16_t key_code;
    int32_t is_repeat;
    /// UTF-8,可空。
    const char *chars;
    const char *chars_ignoring_modifiers;
} WarpCefOsrInputEvent;

/// 宿主右键菜单命令(与 Rust 侧 `OSR_MENU_*` 一一对应)。
enum {
    WARP_CEF_OSR_MENU_RELOAD = 0,
    WARP_CEF_OSR_MENU_INSPECT = 1,
};

/// 输入法动作(与 Rust 侧 `WarpCefOsrImeAction` 一一对应)。
enum {
    WARP_CEF_OSR_IME_SET_COMPOSITION = 0,
    WARP_CEF_OSR_IME_COMMIT = 1,
    WARP_CEF_OSR_IME_FINISH = 2,
    WARP_CEF_OSR_IME_CANCEL = 3,
};

/// 一次按键的完整上下文(deferred 模型):基础键字段 + `interpretKeyEvents:` 期间累积的
/// 输入法状态。**决策**(发 KEYDOWN+CHAR 还是 commit/composition)放在 Rust 侧 ——
/// 那里有 CEF 绑定,且纯逻辑可以单测(参考实现 CefSwift / cefclient 的
/// HandleKeyEventBefore/AfterTextInputClient 同款拆分)。
typedef struct {
    uint32_t modifier_flags;
    uint16_t key_code;
    int32_t is_repeat;
    /// event.characters / charactersIgnoringModifiers(UTF-8,可空)。
    const char *chars;
    const char *chars_ignoring_modifiers;
    /// `interpretKeyEvents:` 期间 `insertText:` 累积的文本(可空 = 没插入)。
    const char *text_to_insert;
    /// 当前 composition 文本(可空 = 无)。
    const char *marked_text;
    int32_t marked_sel_from;
    int32_t marked_sel_to;
    /// -1 = 没有要替换的既有文本(NSNotFound)。
    int32_t replacement_from;
    int32_t replacement_to;
    int32_t old_has_marked;
    int32_t has_marked;
    int32_t unmark_called;
} WarpCefOsrKeyInput;

/// 输入法在按键之外直接发起的调用(候选点击上屏、unmarkText 等)。
typedef struct {
    int32_t action;
    const char *text;
    int32_t sel_from;
    int32_t sel_to;
    int32_t replacement_from;
    int32_t replacement_to;
    int32_t keep_selection;
} WarpCefOsrImeCommand;

typedef struct {
    void (*handle_event)(uint64_t webview_id, const WarpCefOsrInputEvent *event);
    void (*handle_edit_command)(uint64_t webview_id, int32_t command);
    void (*handle_focus)(uint64_t webview_id, int32_t focused);
    void (*handle_key)(uint64_t webview_id, const WarpCefOsrKeyInput *key);
    void (*handle_ime)(uint64_t webview_id, const WarpCefOsrImeCommand *command);
    /// 宿主右键菜单命令(`warpShowContextMenu:` 里选中的项)→ Rust 走 CEF API。
    void (*handle_menu_command)(uint64_t webview_id, int32_t command, double x, double y);
} WarpCefOsrInputCallbacks;

static WarpCefOsrInputCallbacks gInputCallbacks;

/// 注册输入回调(创建浏览器之前调用;Rust 侧传的是静态函数指针,不会悬垂)。
void warp_cef_osr_view_set_input_callbacks(const WarpCefOsrInputCallbacks *callbacks) {
    if (callbacks != NULL) {
        gInputCallbacks = *callbacks;
    }
}

/// 光标语义:Rust 侧把 CEF 的 `cef_cursor_type_t` 归一化后传过来,
/// 避免在 ObjC 里重复一份 CEF 枚举值。
enum {
    WARP_CEF_OSR_CURSOR_ARROW = 0,
    WARP_CEF_OSR_CURSOR_HAND = 1,
    WARP_CEF_OSR_CURSOR_IBEAM = 2,
    WARP_CEF_OSR_CURSOR_CROSSHAIR = 3,
    WARP_CEF_OSR_CURSOR_RESIZE_H = 4,
    WARP_CEF_OSR_CURSOR_RESIZE_V = 5,
    WARP_CEF_OSR_CURSOR_GRAB = 6,
    WARP_CEF_OSR_CURSOR_GRABBING = 7,
    WARP_CEF_OSR_CURSOR_NOT_ALLOWED = 8,
    WARP_CEF_OSR_CURSOR_ZOOM_IN = 9,
    WARP_CEF_OSR_CURSOR_ZOOM_OUT = 10,
    WARP_CEF_OSR_CURSOR_MOVE = 11,
    WARP_CEF_OSR_CURSOR_DISAPPEAR = 12,
};

static NSCursor *WarpCefOsrCursorForSemantic(int32_t semantic) {
    switch (semantic) {
        case WARP_CEF_OSR_CURSOR_HAND:
            return [NSCursor pointingHandCursor];
        case WARP_CEF_OSR_CURSOR_IBEAM:
            return [NSCursor IBeamCursor];
        case WARP_CEF_OSR_CURSOR_CROSSHAIR:
            return [NSCursor crosshairCursor];
        case WARP_CEF_OSR_CURSOR_RESIZE_H:
            return [NSCursor resizeLeftRightCursor];
        case WARP_CEF_OSR_CURSOR_RESIZE_V:
            return [NSCursor resizeUpDownCursor];
        case WARP_CEF_OSR_CURSOR_GRAB:
        case WARP_CEF_OSR_CURSOR_MOVE:
            return [NSCursor openHandCursor];
        case WARP_CEF_OSR_CURSOR_GRABBING:
            return [NSCursor closedHandCursor];
        case WARP_CEF_OSR_CURSOR_NOT_ALLOWED:
            return [NSCursor operationNotAllowedCursor];
        case WARP_CEF_OSR_CURSOR_ZOOM_IN:
            // zoomIn/OutCursor 是 macOS 15+;老系统退回箭头。
            if (@available(macOS 15.0, *)) {
                return [NSCursor zoomInCursor];
            }
            return [NSCursor arrowCursor];
        case WARP_CEF_OSR_CURSOR_ZOOM_OUT:
            if (@available(macOS 15.0, *)) {
                return [NSCursor zoomOutCursor];
            }
            return [NSCursor arrowCursor];
        case WARP_CEF_OSR_CURSOR_DISAPPEAR:
            return [NSCursor disappearingItemCursor];
        case WARP_CEF_OSR_CURSOR_ARROW:
        default:
            return [NSCursor arrowCursor];
    }
}

/// OSR 宿主视图:把 CEF 送来的帧贴到自己的 CALayer 上,并**转发 AppKit 输入事件**。
///
/// 关键契约(cef_types.h):accelerated paint 的共享句柄"每帧可能不同、不可缓存、
/// 回调返回即回收",所以每帧都要重贴,并且必须放在 CATransaction 里禁用隐式动画,
/// 否则会出现拖影/闪烁(T1 探针实测同款写法)。
///
/// 输入为什么必须自己转发:windowless 下 CEF 没有自己的原生视图,AppKit 的事件不会
/// 自动流进渲染器 —— 鼠标/滚轮/键盘/焦点/编辑命令都得宿主实现
/// (CEFSWIFT-EVALUATION.md §1.4;参考 cefclient 的 mac OSR 客户端)。
///
/// 坐标系:本视图**不是 flipped**(与 WebViewContainerView 一致),frame 由 Rust 侧
/// 用与 wry/CEF 子视图相同的翻转公式算好;事件坐标在这里翻成 CEF 的左上原点。
@interface WarpCefOsrView : NSView <NSTextInputClient> {
  @public
    /// 创建时由 Rust 侧写入:回调据此定位是哪个 webview。
    uint64_t _webviewId;
    NSTrackingArea *_trackingArea;
    /// 页面内容的 CALayer(原实现直接写 self.layer.contents;T6 起拆成独立子层,
    /// 以便给 `<select>` 这类弹层单独一层叠加在上面)。
    CALayer *_contentLayer;
    /// 最近一次右键位置(DIP、左上原点):"检查元素"用它把 DevTools 定位到点中的元素。
    NSPoint _lastContextMenuPointDIP;
    /// 弹层(`<select>` 下拉、自动填充等):尺寸来自 on_popup_size(DIP、左上原点),
    /// 显隐来自 on_popup_show,绘制走 POPUP 类型的 paint 回调。
    CALayer *_popupLayer;
    /// 页面请求的光标语义(见 WARP_CEF_OSR_CURSOR_*)。
    int32_t _cursorSemantic;
    /// 滚轮亚像素余量:CEF 只收整数增量,不留余量会把慢速触控板滚动吃掉。
    double _scrollResidualX;
    double _scrollResidualY;
    /// 候选框锚点:CEF 经 on_ime_composition_range_changed 回报的 composition 几何
    /// (DIP、**左上原点**;本视图非 flipped,firstRect 里再翻成屏幕坐标)。
    NSRect _imeBoundsDIP;
    BOOL _hasImeBounds;
    /// composition 期间 CEF 回报的选区(UTF-16),答 hasMarkedText/selectedRange 用。
    NSRange _cefSelectedRange;
    BOOL _hasCefSelection;
    /// 程序化焦点同步期间置位:`makeFirstResponder:` 会**同步**回调
    /// `becomeFirstResponder`/`resignFirstResponder`,若那时再通知 Rust
    /// (`focus()` → 又调回 `warp_cef_osr_view_focus`)就会递归,故这两处回调要看它。
    BOOL _syncingFocus;
    /// deferred 模型:`interpretKeyEvents:` 期间累积的状态(见 WarpCefOsrKeyInput)。
    BOOL _handlingKeyDown;
    NSMutableString *_textToInsert;
    NSString *_markedText;
    BOOL _hasMarkedText;
    BOOL _oldHasMarkedText;
    NSRange _markedSelectionRange;
    NSRange _markedReplacementRange;
    BOOL _unmarkTextCalled;
}
@end

@implementation WarpCefOsrView

- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        self.wantsLayer = YES;
        // 层树(T6):root 透明、**不裁剪**(弹层可能伸出视图边界),下挂内容层与弹层。
        CALayer *root = [CALayer layer];
        root.backgroundColor = NSColor.clearColor.CGColor;
        root.opaque = NO;
        root.masksToBounds = NO;
        // surface 的像素尺寸 = 点数 × contentsScale,resize 后正好铺满。
        _contentLayer = [[CALayer alloc] init];
        _contentLayer.contentsGravity = kCAGravityResize;
        _contentLayer.frame = self.bounds;
        _popupLayer = [[CALayer alloc] init];
        _popupLayer.contentsGravity = kCAGravityResize;
        _popupLayer.hidden = YES;
        [root addSublayer:_contentLayer];
        [root addSublayer:_popupLayer];
        self.layer = root;
        _cursorSemantic = WARP_CEF_OSR_CURSOR_ARROW;
        _textToInsert = [[NSMutableString alloc] init];
        _markedReplacementRange = NSMakeRange(NSNotFound, 0);
        _markedSelectionRange = NSMakeRange(NSNotFound, 0);
    }
    return self;
}

- (void)dealloc {
    if (_trackingArea != nil) {
        [self removeTrackingArea:_trackingArea];
        [_trackingArea release];
        _trackingArea = nil;
    }
    [_textToInsert release];
    [_markedText release];
    [_contentLayer release];
    [_popupLayer release];
    [super dealloc];
}

#pragma mark 输入转发

- (BOOL)acceptsFirstResponder {
    return YES;
}

/// 窗口未激活时的第一次点击也要落到页面上(与 WKWebView/参考实现一致):
/// 否则用户从别的 app 切回来点输入框,那一下只用来激活窗口,页面收不到点击。
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    return YES;
}

/// 视图尺寸变化时内容层跟着铺满(弹层的位置由 CEF 的 on_popup_size 给,不在此调整)。
- (void)setFrameSize:(NSSize)newSize {
    [super setFrameSize:newSize];
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    _contentLayer.frame = self.bounds;
    [CATransaction commit];
}

- (BOOL)becomeFirstResponder {
    // 让 CEF 知道浏览器获得焦点:否则页面里没有光标、键盘/IME 也不工作。
    // `_syncingFocus` 期间不回调:那是 Rust 的 focus() 主动设焦点,CEF 侧由它自己
    // 同步(见 warp_cef_osr_view_focus),回调会绕回 focus() 造成递归。
    if (!_syncingFocus && gInputCallbacks.handle_focus != NULL) {
        gInputCallbacks.handle_focus(_webviewId, 1);
    }
    return [super becomeFirstResponder];
}

- (BOOL)resignFirstResponder {
    if (!_syncingFocus && gInputCallbacks.handle_focus != NULL) {
        gInputCallbacks.handle_focus(_webviewId, 0);
    }
    return [super resignFirstResponder];
}

/// 视图坐标(**左上原点、DIP**)—— CEF 鼠标事件的口径。
- (NSPoint)warpDipPointForEvent:(NSEvent *)event {
    NSPoint inView = [self convertPoint:event.locationInWindow fromView:nil];
    return NSMakePoint(inView.x, self.bounds.size.height - inView.y);
}

- (void)warpSendEvent:(WarpCefOsrInputEvent *)event {
    if (gInputCallbacks.handle_event != NULL) {
        gInputCallbacks.handle_event(_webviewId, event);
    }
}

- (void)warpSendMouse:(NSEvent *)event
                 type:(int32_t)type
               button:(int32_t)button
                 isUp:(BOOL)isUp
              leaving:(BOOL)leaving {
    NSPoint point = [self warpDipPointForEvent:event];
    WarpCefOsrInputEvent out = {0};
    out.type = type;
    out.x = point.x;
    out.y = point.y;
    out.modifier_flags = (uint32_t)event.modifierFlags;
    // clickCount 只对鼠标按键事件有效:对 enter/exit/move 取它会抛 ObjC 异常
    // (实测 mouseEntered → -[NSEvent clickCount] 直接崩掉进程)。
    out.click_count =
        (type == WARP_CEF_OSR_EVENT_MOUSE_CLICK) ? (int32_t)event.clickCount : 0;
    out.button = button;
    out.mouse_up = isUp ? 1 : 0;
    out.mouse_leave = leaving ? 1 : 0;
    [self warpSendEvent:&out];
}

- (void)mouseDown:(NSEvent *)event {
    // 点进页面即接管键盘焦点(与真实浏览器一致):不切 first responder 的话键盘输入
    // 仍留在 warpui 宿主视图上,页面收不到任何按键。
    if (self.window.firstResponder != self) {
        [self.window makeFirstResponder:self];
    }
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:0 isUp:NO leaving:NO];
}

- (void)mouseUp:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:0 isUp:YES leaving:NO];
}

- (void)mouseDragged:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:0 isUp:NO leaving:NO];
}

/// 宿主右键菜单(异步 NSMenu)。
///
/// **为什么不用 CEF 的原生菜单**:OSR 下这条链走不通 —— `CefMenuManager::CreateContextMenu`
/// (即 `on_before_context_menu` 的唯一调用点)在实测里一次都没被调用过,菜单自然不出现。
/// 按 OSR-PLAN.md T6 的约定由宿主自己弹面板,选中项经回调回 Rust 走 CEF API。
- (void)warpShowContextMenu:(NSEvent *)event {
    NSPoint inView = [self convertPoint:event.locationInWindow fromView:nil];
    _lastContextMenuPointDIP = NSMakePoint(inView.x, self.bounds.size.height - inView.y);

    NSMenu *menu = [[NSMenu alloc] initWithTitle:@""];
    menu.autoenablesItems = NO;
    NSMenuItem *reload = [[NSMenuItem alloc] initWithTitle:@"重新加载"
                                                   action:@selector(warpMenuReload:)
                                            keyEquivalent:@""];
    reload.target = self;
    NSMenuItem *inspect = [[NSMenuItem alloc] initWithTitle:@"检查元素"
                                                    action:@selector(warpMenuInspect:)
                                             keyEquivalent:@""];
    inspect.target = self;
    [menu addItem:reload];
    [menu addItem:inspect];
    // 位置用视图坐标(inView:self),避免自己换算屏幕坐标。
    [menu popUpMenuPositioningItem:nil atLocation:inView inView:self];
    [reload release];
    [inspect release];
    [menu release];
}

- (void)warpSendMenuCommand:(int32_t)command {
    if (gInputCallbacks.handle_menu_command == NULL) {
        return;
    }
    gInputCallbacks.handle_menu_command(_webviewId, command, _lastContextMenuPointDIP.x,
                                        _lastContextMenuPointDIP.y);
}

- (void)warpMenuReload:(id)sender {
    (void)sender;
    [self warpSendMenuCommand:WARP_CEF_OSR_MENU_RELOAD];
}

- (void)warpMenuInspect:(id)sender {
    (void)sender;
    [self warpSendMenuCommand:WARP_CEF_OSR_MENU_INSPECT];
}

- (void)rightMouseDown:(NSEvent *)event {
    if (self.window.firstResponder != self) {
        [self.window makeFirstResponder:self];
    }
    // 先照常把右键转发给页面(页面的 contextmenu JS 事件/自定义菜单照旧),再弹宿主菜单。
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:1 isUp:NO leaving:NO];
    [self warpShowContextMenu:event];
}

- (void)rightMouseUp:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:1 isUp:YES leaving:NO];
}

- (void)rightMouseDragged:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:1 isUp:NO leaving:NO];
}

- (void)otherMouseDown:(NSEvent *)event {
    // 中键/侧键:统一按中键转发,页面自己决定语义。
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:2 isUp:NO leaving:NO];
}

- (void)otherMouseUp:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_CLICK button:2 isUp:YES leaving:NO];
}

- (void)otherMouseDragged:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:2 isUp:NO leaving:NO];
}

- (void)mouseMoved:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:-1 isUp:NO leaving:NO];
}

- (void)mouseEntered:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:-1 isUp:NO leaving:NO];
}

- (void)mouseExited:(NSEvent *)event {
    [self warpSendMouse:event type:WARP_CEF_OSR_EVENT_MOUSE_MOVE button:-1 isUp:NO leaving:YES];
}

/// 滚轮:优先用 CGEvent 的 point delta(cefclient 同源,像素级且带惯性),
/// 没有时退回 NSEvent 的 line delta(非精确设备 ×40 换算成像素)。
- (void)scrollWheel:(NSEvent *)event {
    double dx = event.scrollingDeltaX;
    double dy = event.scrollingDeltaY;
    CGEventRef cgEvent = [event CGEvent];
    if (cgEvent != NULL) {
        double pointY =
            (double)CGEventGetIntegerValueField(cgEvent, kCGScrollWheelEventPointDeltaAxis1);
        double pointX =
            (double)CGEventGetIntegerValueField(cgEvent, kCGScrollWheelEventPointDeltaAxis2);
        if (pointX != 0 || pointY != 0) {
            dx = pointX;
            dy = pointY;
        } else if (!event.hasPreciseScrollingDeltas) {
            dx *= 40;
            dy *= 40;
        }
    } else if (!event.hasPreciseScrollingDeltas) {
        dx *= 40;
        dy *= 40;
    }
    // 亚像素余量跨事件累加,避免慢速滚动被取整吃掉(参考实现同款)。
    double totalX = dx + _scrollResidualX;
    double totalY = dy + _scrollResidualY;
    int32_t sendX = (int32_t)trunc(totalX);
    int32_t sendY = (int32_t)trunc(totalY);
    _scrollResidualX = totalX - (double)sendX;
    _scrollResidualY = totalY - (double)sendY;
    if (sendX == 0 && sendY == 0) {
        return;
    }
    NSPoint point = [self warpDipPointForEvent:event];
    WarpCefOsrInputEvent out = {0};
    out.type = WARP_CEF_OSR_EVENT_MOUSE_WHEEL;
    out.x = point.x;
    out.y = point.y;
    out.modifier_flags = (uint32_t)event.modifierFlags;
    out.delta_x = sendX;
    out.delta_y = sendY;
    [self warpSendEvent:&out];
}

/// 键盘:只做原始采集(键码 + 字符),KEYDOWN/CHAR 两段式由 Rust 侧决定。
///
/// `characters`/`isARepeat` 只对 keyDown/keyUp 有效(cefclient 的 getKeyEvent 同样
/// 只在这两类事件里读),flagsChanged 只带键码与修饰键位。
- (void)warpSendKey:(NSEvent *)event type:(int32_t)keyType {
    WarpCefOsrInputEvent out = {0};
    out.type = WARP_CEF_OSR_EVENT_KEY;
    out.modifier_flags = (uint32_t)event.modifierFlags;
    out.key_type = keyType;
    out.key_code = (uint16_t)event.keyCode;
    if (keyType == 0 || keyType == 1) {
        NSString *chars = event.characters;
        NSString *unmodified = event.charactersIgnoringModifiers;
        out.is_repeat = event.isARepeat ? 1 : 0;
        out.chars = chars.length > 0 ? chars.UTF8String : NULL;
        out.chars_ignoring_modifiers = unmodified.length > 0 ? unmodified.UTF8String : NULL;
    }
    [self warpSendEvent:&out];
}

/// 键盘按下(deferred 模型,参考 cefclient 的 `CefTextInputClientOSRMac`):
/// 先让输入法经 `interpretKeyEvents:` 回调本类的 `setMarkedText:`/`insertText:`/
/// `unmarkText` —— 那些回调**只累积状态**;本方法末尾把完整上下文交给 Rust 决策:
/// 普通按键 → KEYDOWN+CHAR(保住 T5 的 JS keydown/光标)、组合中 → ime_set_composition、
/// 上屏 → ime_commit_text。**只有这样做才能同时满足"英文键位保真"与"中文能上屏"**:
/// 直接转发 `insertText:` 会让普通字母也走 ime_commit_text(页面收不到 keydown)。
- (void)keyDown:(NSEvent *)event {
    _oldHasMarkedText = _hasMarkedText;
    _handlingKeyDown = YES;
    [_textToInsert setString:@""];
    _markedReplacementRange = NSMakeRange(NSNotFound, 0);
    _unmarkTextCalled = NO;
    [self interpretKeyEvents:@[ event ]];
    _handlingKeyDown = NO;

    WarpCefOsrKeyInput key = {0};
    key.modifier_flags = (uint32_t)event.modifierFlags;
    key.key_code = (uint16_t)event.keyCode;
    key.is_repeat = event.isARepeat ? 1 : 0;
    NSString *chars = event.characters;
    NSString *unmodified = event.charactersIgnoringModifiers;
    key.chars = chars.length > 0 ? chars.UTF8String : NULL;
    key.chars_ignoring_modifiers = unmodified.length > 0 ? unmodified.UTF8String : NULL;
    key.text_to_insert = _textToInsert.length > 0 ? _textToInsert.UTF8String : NULL;
    key.marked_text = _markedText.length > 0 ? _markedText.UTF8String : NULL;
    key.marked_sel_from = (int32_t)_markedSelectionRange.location;
    key.marked_sel_to = (int32_t)(_markedSelectionRange.location + _markedSelectionRange.length);
    key.replacement_from = (_markedReplacementRange.location == NSNotFound)
                               ? -1
                               : (int32_t)_markedReplacementRange.location;
    key.replacement_to = (_markedReplacementRange.location == NSNotFound)
                             ? -1
                             : (int32_t)(_markedReplacementRange.location +
                                         _markedReplacementRange.length);
    key.old_has_marked = _oldHasMarkedText ? 1 : 0;
    key.has_marked = _hasMarkedText ? 1 : 0;
    key.unmark_called = _unmarkTextCalled ? 1 : 0;
    if (gInputCallbacks.handle_key != NULL) {
        gInputCallbacks.handle_key(_webviewId, &key);
    }
    _markedReplacementRange = NSMakeRange(NSNotFound, 0);
}

- (void)keyUp:(NSEvent *)event {
    [self warpSendKey:event type:1];
}

- (void)flagsChanged:(NSEvent *)event {
    [self warpSendKey:event type:2];
}

#pragma mark 输入法(NSTextInputClient)

/// 转发一条输入法动作给 Rust(按键外的直接调用:候选点击上屏、unmark 等)。
- (void)warpSendIme:(int32_t)action
               text:(NSString *)text
         selection:(NSRange)selection
     replacement:(NSRange)replacement {
    if (gInputCallbacks.handle_ime == NULL) {
        return;
    }
    WarpCefOsrImeCommand command = {0};
    command.action = action;
    command.text = text.length > 0 ? text.UTF8String : NULL;
    command.sel_from = (selection.location == NSNotFound) ? -1 : (int32_t)selection.location;
    command.sel_to = (selection.location == NSNotFound)
                         ? -1
                         : (int32_t)(selection.location + selection.length);
    command.replacement_from =
        (replacement.location == NSNotFound) ? -1 : (int32_t)replacement.location;
    command.replacement_to = (replacement.location == NSNotFound)
                                 ? -1
                                 : (int32_t)(replacement.location + replacement.length);
    // keep_selection=1 取自参考实现 CefSwift(其 imeFinishComposing 默认
    // keepSelection: true);cefclient 的同名调用传 false,两者不一致,这里跟随
    // 我们的对照实现。**注意**:deferred(按键内)那条路径用的是 0,与 CefSwift 一致。
    command.keep_selection = 1;
    gInputCallbacks.handle_ime(_webviewId, &command);
}

/// composition 更新(拼音串)。按键期间只累积;按键之外(输入法自发组合)直接转发。
- (void)setMarkedText:(id)string
        selectedRange:(NSRange)selectedRange
     replacementRange:(NSRange)replacementRange {
    NSString *text = [string isKindOfClass:[NSAttributedString class]]
                         ? [(NSAttributedString *)string string]
                         : (NSString *)string;
    if (![text isKindOfClass:[NSString class]]) {
        text = @"";
    }
    [_markedText release];
    _markedText = [text copy];
    _hasMarkedText = text.length > 0;
    _markedSelectionRange = selectedRange;
    if (_handlingKeyDown) {
        // 与 KEYDOWN/CHAR 的时序一起交给 Rust 决定(见 keyDown)。
        _markedReplacementRange = replacementRange;
        return;
    }
    if (text.length == 0) {
        // 空串 = 输入法取消 composition。
        [self warpSendIme:WARP_CEF_OSR_IME_CANCEL
                     text:nil
                selection:NSMakeRange(NSNotFound, 0)
              replacement:NSMakeRange(NSNotFound, 0)];
        return;
    }
    [self warpSendIme:WARP_CEF_OSR_IME_SET_COMPOSITION
                 text:text
            selection:selectedRange
          replacement:replacementRange];
}

/// 上屏(候选提交/直接键入)。按键期间只累积(普通字母不能走 commit)。
- (void)insertText:(id)string replacementRange:(NSRange)replacementRange {
    NSString *text = [string isKindOfClass:[NSAttributedString class]]
                         ? [(NSAttributedString *)string string]
                         : (NSString *)string;
    if (![text isKindOfClass:[NSString class]] || text.length == 0) {
        return;
    }
    if (_handlingKeyDown) {
        [_textToInsert appendString:text];
    } else {
        [self warpSendIme:WARP_CEF_OSR_IME_COMMIT
                     text:text
                selection:NSMakeRange(NSNotFound, 0)
              replacement:replacementRange];
    }
    // 插入文本必然清掉 composition(参考实现同结论)。
    _hasMarkedText = NO;
    [_markedText release];
    _markedText = nil;
}

- (void)unmarkText {
    _hasMarkedText = NO;
    [_markedText release];
    _markedText = nil;
    if (_handlingKeyDown) {
        _unmarkTextCalled = YES;
        return;
    }
    [self warpSendIme:WARP_CEF_OSR_IME_FINISH
                 text:nil
            selection:NSMakeRange(NSNotFound, 0)
          replacement:NSMakeRange(NSNotFound, 0)];
}

- (BOOL)hasMarkedText {
    return _hasMarkedText && _markedText.length > 0;
}

- (NSRange)markedRange {
    return [self hasMarkedText] ? NSMakeRange(0, _markedText.length) : NSMakeRange(NSNotFound, 0);
}

/// CEF 只在 composition 变化时回报选区(on_ime_composition_range_changed);
/// 非 composition 的文档选区需要 on_text_selection_changed,暂未接。
- (NSRange)selectedRange {
    return _hasCefSelection ? _cefSelectedRange : NSMakeRange(NSNotFound, 0);
}

- (NSAttributedString *)attributedSubstringForProposedRange:(NSRange)range
                                               actualRange:(NSRangePointer)actualRange {
    (void)range;
    (void)actualRange;
    return nil;
}

/// 中日韩输入法靠这些属性给 composition 分词/画下划线;返回空数组时部分输入法会判定
/// "客户端不支持 composition",把按键原样透传(**实测微信输入法如此**)。属性集与 CEF 自带
/// 的 mac OSR 客户端一致(它带的 NSTextInputReplacementRangeAttributeName 在 SDK 头文件里
/// 没声明,故不带;分词依赖的是 NSMarkedClauseSegmentAttributeName)。
- (NSArray<NSAttributedStringKey> *)validAttributesForMarkedText {
    return @[
        NSUnderlineStyleAttributeName, NSUnderlineColorAttributeName,
        NSMarkedClauseSegmentAttributeName
    ];
}

/// 候选框/符号面板的锚点(屏幕坐标)。数据来自 CEF 的
/// `on_ime_composition_range_changed`(composition 期间逐字的位置,DIP/左上原点)。
- (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actualRange {
    if (actualRange != NULL) {
        *actualRange = range;
    }
    // 没有 composition 几何时锚在视图左上角,而不是屏幕原点。
    NSRect caret = _hasImeBounds ? _imeBoundsDIP : NSMakeRect(4, 4, 1, 16);
    // DIP(左上原点、y 向下)→ 视图坐标(左下原点、y 向上):本视图非 flipped。
    NSRect viewRect = NSMakeRect(caret.origin.x,
                                 self.bounds.size.height - (caret.origin.y + caret.size.height),
                                 MAX(caret.size.width, 1.0), MAX(caret.size.height, 1.0));
    NSRect inWindow = [self convertRect:viewRect toView:nil];
    return self.window ? [self.window convertRectToScreen:inWindow] : inWindow;
}

- (NSUInteger)characterIndexForPoint:(NSPoint)point {
    (void)point;
    return NSNotFound;
}

/// 快捷键拦截(**排在 AppKit 菜单之前**):只认 Cmd+A/C/V/X/Z,其余一律交回
/// `[super performKeyEquivalent:]`,zap 自己的 Cmd+T/Cmd+1..9 等不受影响。
///
/// 为什么需要它:zap 的 `WarpWindow::performKeyEquivalent:` 对嵌入视图用的是
/// `mods == NSEventModifierFlagCommand` **精确比较**,开着 CapsLock 时会整块跳过
/// (实测 ns_flags 带 0x10000),这组编辑快捷键就落到菜单上(Warp 自己的 Copy/Paste
/// CustomAction)或被当普通键发给页面 —— 页面里的复制/全选/剪切随之失效。
/// 这里用 **设备无关掩码** 判定修饰键(低位是设备相关位,实测每次按键都带 0x100/0x8)。
- (BOOL)performKeyEquivalent:(NSEvent *)event {
    if (event.type != NSEventTypeKeyDown || self.window.firstResponder != self) {
        return [super performKeyEquivalent:event];
    }
    NSUInteger mods = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    if ((mods & NSEventModifierFlagCommand) == 0) {
        return [super performKeyEquivalent:event];
    }
    NSUInteger others = mods & ~(NSEventModifierFlagCommand | NSEventModifierFlagShift |
                                 NSEventModifierFlagCapsLock);
    if (others != 0) {
        return [super performKeyEquivalent:event];
    }
    NSString *key = event.charactersIgnoringModifiers.lowercaseString;
    int32_t command = -1;
    BOOL shift = (mods & NSEventModifierFlagShift) != 0;
    if ([key isEqualToString:@"a"]) {
        command = WARP_CEF_OSR_EDIT_SELECT_ALL;
    } else if ([key isEqualToString:@"c"]) {
        command = WARP_CEF_OSR_EDIT_COPY;
    } else if ([key isEqualToString:@"x"]) {
        command = WARP_CEF_OSR_EDIT_CUT;
    } else if ([key isEqualToString:@"v"]) {
        command = WARP_CEF_OSR_EDIT_PASTE;
    } else if ([key isEqualToString:@"z"]) {
        command = shift ? WARP_CEF_OSR_EDIT_REDO : WARP_CEF_OSR_EDIT_UNDO;
    }
    if (command < 0) {
        return [super performKeyEquivalent:event];
    }
    // 转发到 focused frame 的编辑命令(Rust 侧有 `edit command N` 调试日志可循)。
    [self warpSendEditCommand:command];
    return YES;
}

/// 输入法未消费的"命令键"(方向键/退格/回车…):**只吞掉**。这些键已经由 keyDown 的
/// after 阶段按普通按键转发给页面了;若调 super,NSResponder 会走 noResponderFor:
/// 触发系统提示音(实测很吵)。
- (void)doCommandBySelector:(SEL)selector {
    (void)selector;
}

#pragma mark 跟踪区域与光标

/// mouseMoved/mouseEntered/mouseExited 只有存在 tracking area 时才会送来。
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    if (_trackingArea != nil) {
        [self removeTrackingArea:_trackingArea];
        [_trackingArea release];
        _trackingArea = nil;
    }
    _trackingArea = [[NSTrackingArea alloc]
        initWithRect:NSZeroRect
             options:(NSTrackingMouseEnteredAndExited | NSTrackingMouseMoved |
                      NSTrackingCursorUpdate | NSTrackingActiveInActiveApp |
                      NSTrackingInVisibleRect)
               owner:self
            userInfo:nil];
    [self addTrackingArea:_trackingArea];
}

- (void)cursorUpdate:(NSEvent *)event {
    // AppKit 每次鼠标移动都会重置光标,故这里持续按页面请求重设。
    [WarpCefOsrCursorForSemantic(_cursorSemantic) set];
}

- (void)resetCursorRects {
    [self addCursorRect:self.bounds cursor:WarpCefOsrCursorForSemantic(_cursorSemantic)];
}

#pragma mark 编辑命令(响应者链)

// Cmd+C/V/X/A 由 WarpWindow::performKeyEquivalent: 的"嵌入视图"分支直接发
// copy:/cut:/paste:/selectAll: 给 first responder(见 crates/warpui/.../window.m),
// 故这里必须实现,否则这些快捷键在页面里静默失效。Cmd+Z/Shift+Cmd+Z 走 warp 自己的
// Edit 菜单(CustomAction),与 windowed 现状一致,故不在这里拦截。
- (void)warpSendEditCommand:(int32_t)command {
    if (gInputCallbacks.handle_edit_command != NULL) {
        gInputCallbacks.handle_edit_command(_webviewId, command);
    }
}

- (void)copy:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_COPY];
}

- (void)cut:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_CUT];
}

- (void)paste:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_PASTE];
}

- (void)selectAll:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_SELECT_ALL];
}

- (void)undo:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_UNDO];
}

- (void)redo:(id)sender {
    [self warpSendEditCommand:WARP_CEF_OSR_EDIT_REDO];
}

@end

/// 让自建宿主视图成为窗口 first responder。
///
/// **OSR 必须自己做这件事**:CEF 的 `SetFocus(true)` 在 mac 上只对 content native view
/// 调 `makeFirstResponder`(browser_platform_delegate_native_mac.mm),windowless 下该
/// view 为 null ⇒ 只调 CEF 的话,键盘事件仍发给 WarpHostView,页面收不到输入(要先用鼠标
/// 点一下页面才行)。wry 后端 `focus()` 内部就是 makeFirstResponder,这里对齐。
/// `focused == 0` 时不动作(与 wry 的 focus() 一致:失焦由 AppKit 自己走响应者链)。
void warp_cef_osr_view_focus(void *view, int32_t focused) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || focused == 0 || v.window == nil) {
        return;
    }
    v->_syncingFocus = YES;
    [v.window makeFirstResponder:v];
    v->_syncingFocus = NO;
}

/// 宿主视图当前是否是窗口的 first responder(加载完成后据此决定要不要补 CEF 焦点:
/// 只有"输入本来就在页面上"时才补,避免把用户在终端里的焦点抢走)。
int32_t warp_cef_osr_view_is_first_responder(void *view) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v.window == nil) {
        return 0;
    }
    return v.window.firstResponder == v ? 1 : 0;
}

/// CEF 回报 composition 的几何与选区(render handler 的
/// on_ime_composition_range_changed 调用):候选框靠它跟随光标。
/// 坐标是 DIP、左上原点,`firstRectForCharacterRange:` 里再换算。
void warp_cef_osr_view_set_ime_bounds(void *view, int32_t sel_from, int32_t sel_to, double x,
                                      double y, double w, double h, int has_bounds) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil) {
        return;
    }
    v->_hasCefSelection = sel_from >= 0 && sel_to >= sel_from;
    v->_cefSelectedRange =
        v->_hasCefSelection ? NSMakeRange((NSUInteger)sel_from, (NSUInteger)(sel_to - sel_from))
                            : NSMakeRange(NSNotFound, 0);
    v->_hasImeBounds = has_bounds != 0;
    v->_imeBoundsDIP = has_bounds ? NSMakeRect(x, y, w, h) : NSZeroRect;
}

/// 页面请求的光标(render handler 的 on_cursor_change 调用)。
void warp_cef_osr_view_set_cursor(void *view, int32_t semantic) {
    if (view == NULL) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    v->_cursorSemantic = semantic;
    // 立即生效一次,并让 AppKit 在后续移动/进入时按 cursor rect 重设。
    [WarpCefOsrCursorForSemantic(semantic) set];
    [v.window invalidateCursorRectsForView:v];
}

/// 创建 OSR 宿主视图并加入容器(坐标 = 容器内、底部原点,与 windowed 子视图同口径)。
/// 返回 +1 持有的指针,销毁时必须交给 [`warp_cef_osr_view_release`] 配平。
void *warp_cef_osr_view_new(void *container, double x, double y, double w, double h,
                            uint64_t webview_id) {
    NSView *parent = (NSView *)container;
    if (parent == nil) {
        return NULL;
    }
    WarpCefOsrView *view = [[WarpCefOsrView alloc] initWithFrame:NSMakeRect(x, y, w, h)];
    view->_webviewId = webview_id;
    [parent addSubview:view];
    // contentsScale 必须在进窗口之后设:窗口的 backingScaleFactor 才是真的 retina 比例。
    CGFloat scale = parent.window ? parent.window.backingScaleFactor : 2.0;
    view.layer.contentsScale = scale;
    // 子层不自动继承父层的 contentsScale,内容层/弹层要各自设(IOSurface 按像素贴)。
    view->_contentLayer.contentsScale = scale;
    view->_popupLayer.contentsScale = scale;
    return (void *)view;
}

/// 贴一帧共享纹理(layer.contents = IOSurface,零拷贝)。
void warp_cef_osr_view_set_surface(void *view, void *surface) {
    if (view == NULL || surface == NULL) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v->_contentLayer.contents = (id)surface;
    [CATransaction commit];
}

/// CPU 兜底(on_paint):把 CEF 的 BGRA 缓冲区包成 CGImage 贴上。
/// 缓冲区与 `view_rect` 同一坐标系(左上原点);CGImage 作为 layer.contents 时
/// 也是左上原点,故不需要翻转(CefSwift/cefclient 同款处理)。
void warp_cef_osr_view_set_bitmap(void *view, const void *buffer, int width, int height,
                                  int bytes_per_row) {
    if (view == NULL || buffer == NULL || width <= 0 || height <= 0) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    CGColorSpaceRef color_space = CGColorSpaceCreateDeviceRGB();
    CGDataProviderRef provider =
        CGDataProviderCreateWithData(NULL, buffer, (size_t)bytes_per_row * (size_t)height, NULL);
    CGImageRef image = CGImageCreate(
        (size_t)width, (size_t)height, 8, 32, (size_t)bytes_per_row, color_space,
        kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst, provider, NULL, false,
        kCGRenderingIntentDefault);
    if (image != NULL) {
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        v->_contentLayer.contents = (id)image;
        [CATransaction commit];
        CGImageRelease(image);
    }
    CGDataProviderRelease(provider);
    CGColorSpaceRelease(color_space);
}

/// 设置 frame(容器坐标)。同样禁用隐式动画,避免跟随布局时出现缩放动画。
void warp_cef_osr_view_set_frame(void *view, double x, double y, double w, double h) {
    if (view == NULL) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v.frame = NSMakeRect(x, y, w, h);
    [CATransaction commit];
}

/// 显示/隐藏(隐藏即"pane 不在可见树中",渲染由 CEF 的 was_hidden 停)。
void warp_cef_osr_view_set_hidden(void *view, int hidden) {
    if (view == NULL) {
        return;
    }
    ((WarpCefOsrView *)view).hidden = hidden ? YES : NO;
}

/// 当前 backing scale(供 CEF 的 ScreenInfo.device_scale_factor 与 DPI 变化检测)。
double warp_cef_osr_view_scale(void *view) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v != nil && v.window != nil) {
        return (double)v.window.backingScaleFactor;
    }
    return 2.0;
}

/// 贴一帧**弹层**内容(与 set_surface 同款,只是落到 popup 层)。
void warp_cef_osr_view_set_popup_surface(void *view, void *surface) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v->_popupLayer == nil) {
        return;
    }
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v->_popupLayer.contents = (id)surface;
    [CATransaction commit];
}

/// 贴一帧**弹层**位图(CPU 兜底路径)。
void warp_cef_osr_view_set_popup_bitmap(void *view, const void *buffer, int width, int height,
                                        int bytesPerRow) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v->_popupLayer == nil || buffer == NULL || width <= 0 || height <= 0) {
        return;
    }
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGDataProviderRef provider = CGDataProviderCreateWithData(NULL, buffer,
                                                              (size_t)bytesPerRow * (size_t)height, NULL);
    CGImageRef image = CGImageCreate(width, height, 8, 32, (size_t)bytesPerRow, colorSpace,
                                     kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst,
                                     provider, NULL, false, kCGRenderingIntentDefault);
    if (image != NULL) {
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        v->_popupLayer.contents = (id)image;
        [CATransaction commit];
        CGImageRelease(image);
    }
    CGDataProviderRelease(provider);
    CGColorSpaceRelease(colorSpace);
}

/// view DIP(左上原点)→ **屏幕坐标**(macOS 屏幕坐标即 DIP,原点在左下)。
///
/// CEF 用它给右键菜单、DevTools、拖拽这些原生 UI 定位;CefRenderHandler 的默认实现
/// 返回 false ⇒ 不实现的话 OSR 下右键菜单根本不会出现(实测)。
int32_t warp_cef_osr_view_screen_point(void *view, double view_x, double view_y, double *out_x,
                                       double *out_y) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v.window == nil || out_x == NULL || out_y == NULL) {
        return 0;
    }
    // DIP 左上原点 → 视图坐标(非 flipped,故 y 翻转)→ 窗口坐标 → 屏幕坐标。
    NSPoint inView = NSMakePoint(view_x, v.bounds.size.height - view_y);
    NSPoint inWindow = [v convertPoint:inView toView:nil];
    NSPoint inScreen = [v.window convertPointToScreen:inWindow];
    *out_x = inScreen.x;
    *out_y = inScreen.y;
    return 1;
}

/// 弹层显隐(on_popup_show)。
void warp_cef_osr_view_show_popup(void *view, int visible) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v->_popupLayer == nil) {
        return;
    }
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v->_popupLayer.hidden = visible == 0;
    [CATransaction commit];
}

/// 弹层位置与尺寸(on_popup_size):**DIP、左上原点**(与 view_rect 同口径);
/// 本视图非 flipped,故 y 翻成底部原点。
void warp_cef_osr_view_set_popup_rect(void *view, double x, double y, double w, double h) {
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    if (v == nil || v->_popupLayer == nil) {
        return;
    }
    CGRect rect = CGRectMake(x, v.bounds.size.height - (y + h), MAX(w, 1.0), MAX(h, 1.0));
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    v->_popupLayer.frame = rect;
    [CATransaction commit];
}

/// 读回当前 layer.contents 的 IOSurface 像素尺寸(证据用:确认 surface =
/// 逻辑点 × scale,没有重复缩放/没糊)。contents 不是 IOSurface(CPU 兜底路径)
/// 或还没贴过帧时写 0。
void warp_cef_osr_view_surface_size(void *view, int *out_width, int *out_height) {
    if (out_width != NULL) {
        *out_width = 0;
    }
    if (out_height != NULL) {
        *out_height = 0;
    }
    if (view == NULL) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    id contents = v->_contentLayer.contents;
    if (contents == nil || CFGetTypeID((CFTypeRef)contents) != IOSurfaceGetTypeID()) {
        return;
    }
    IOSurfaceRef surface = (IOSurfaceRef)contents;
    if (out_width != NULL) {
        *out_width = (int)IOSurfaceGetWidth(surface);
    }
    if (out_height != NULL) {
        *out_height = (int)IOSurfaceGetHeight(surface);
    }
}

/// 销毁:摘出父视图并释放(与 `_new` 的 +1 配平)。
void warp_cef_osr_view_release(void *view) {
    if (view == NULL) {
        return;
    }
    WarpCefOsrView *v = (WarpCefOsrView *)view;
    v->_contentLayer.contents = nil;
    v->_popupLayer.contents = nil;
    [v removeFromSuperview];
    [v release];
}
