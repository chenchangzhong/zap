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
#import <Carbon/Carbon.h>
#import <objc/runtime.h>
#include <stdint.h>

static NSView *gOsrView = nil;
static NSTimer *gBeginFrameTimer = nil;

#pragma mark - IME 回调(Rust 侧实现,这里只存函数指针)

/// T2:windowless 模式下没有 CEF 自建的浏览器视图,`NSTextInputClient` 必须由宿主 NSView
/// 自己实现,再把这些调用翻译成 `CefBrowserHost::ImeSetComposition`/`ImeCommitText`
/// (参考 CEFSWIFT-EVALUATION.md §1.4 的 IME 映射)。用函数指针而不是直接链接 Rust 符号,
/// 保持本文件只依赖 AppKit。
typedef struct {
    void (*set_composition)(const char *utf8, int32_t sel_from, int32_t sel_to,
                            int32_t rep_from, int32_t rep_to);
    void (*commit_text)(const char *utf8, int32_t rep_from, int32_t rep_to);
    void (*finish_composing)(int32_t keep_selection);
    void (*cancel_composition)(void);
} OsrImeCallbacks;

static OsrImeCallbacks gImeCallbacks;

void osr_host_view_set_ime_callbacks(const OsrImeCallbacks *callbacks) {
    if (callbacks != NULL) {
        gImeCallbacks = *callbacks;
    }
}

#pragma mark - 宿主视图(NSTextInputClient)

/// T2 宿主视图:除了贴 IOSurface(T1),还要承接输入法。
/// 注意本视图**不是** flipped 视图(T1 的 layer.contents 方向已按此验证),所以 CEF 给的
/// DIP 几何(左上原点、y 向下)要在 firstRectForCharacterRange: 里手工翻成视图坐标。
@interface OsrHostView : NSView <NSTextInputClient> {
  @public
    NSString *_markedText;        // 当前 composition 文本(供 hasMarkedText/markedRange 回答)
    NSRange _markedRange;         // marked range(UTF-16);无 composition 时 location=NSNotFound
    NSRange _cefSelectedRange;    // CEF 回报的 composition 选区(UTF-16)
    BOOL _hasCefSelection;
    NSRect _caretRectDIP;         // on_ime_composition_range_changed 的首字框(DIP、左上原点)
    BOOL _hasCaretRect;
}
@end

@implementation OsrHostView

- (BOOL)acceptsFirstResponder {
    return YES;
}

/// 键盘事件先交给输入法(`interpretKeyEvents:` 内部走 NSTextInputContext),
/// 输入法要么回调本类的 setMarkedText:/insertText:,要么交给 doCommandBySelector:。
/// T2 不向 CEF 转发普通按键(T5 补 KEYDOWN+CHAR 两段式),故此处不再往下传。
- (void)keyDown:(NSEvent *)event {
    printf("[osr] keyDown keyCode=%d chars=%s\n", (int)event.keyCode,
           event.characters.length ? event.characters.UTF8String : "(none)");
    fflush(stdout);
    [self interpretKeyEvents:@[ event ]];
}

/// 输入法未消费的"命令键"(方向键、退格…):吞掉。NSResponder 的默认实现会走
/// noResponderFor: 触发系统提示音,逐一实测输入法时会很吵。
- (void)doCommandBySelector:(SEL)selector {
    printf("[osr] doCommandBySelector %s(T2 只吞掉,不转发)\n", sel_getName(selector));
    fflush(stdout);
}

#pragma mark NSTextInputClient

/// 拼音串(composition 更新)→ CEF ime_set_composition。
- (void)setMarkedText:(id)string
        selectedRange:(NSRange)selectedRange
     replacementRange:(NSRange)replacementRange {
    NSString *text = [string isKindOfClass:[NSAttributedString class]]
                         ? [(NSAttributedString *)string string]
                         : (NSString *)string;
    if (![text isKindOfClass:[NSString class]]) {
        text = @"";
    }
    _markedText = [text copy];
    _markedRange = text.length > 0 ? NSMakeRange(0, text.length) : NSMakeRange(NSNotFound, 0);
    // NSNotFound ⇒ 本次没有要替换的既有文本,CEF 侧用 null 表示。
    int32_t rep_from = (replacementRange.location == NSNotFound)
                           ? -1
                           : (int32_t)replacementRange.location;
    int32_t rep_to = (replacementRange.location == NSNotFound)
                         ? -1
                         : (int32_t)(replacementRange.location + replacementRange.length);
    printf("[osr] NSTextInputClient setMarkedText \"%s\" sel={%lu,%lu} rep={%ld,%lu}\n",
           text.UTF8String, (unsigned long)selectedRange.location,
           (unsigned long)selectedRange.length, (long)replacementRange.location,
           (unsigned long)replacementRange.length);
    fflush(stdout);
    if (text.length == 0) {
        // 空串=清空 composition(输入法取消/退格到空)。
        if (gImeCallbacks.cancel_composition) {
            gImeCallbacks.cancel_composition();
        }
        return;
    }
    if (gImeCallbacks.set_composition) {
        gImeCallbacks.set_composition(text.UTF8String, (int32_t)selectedRange.location,
                                      (int32_t)(selectedRange.location + selectedRange.length),
                                      rep_from, rep_to);
    }
}

/// 上屏(候选提交、直接键入)→ CEF ime_commit_text。
- (void)insertText:(id)string replacementRange:(NSRange)replacementRange {
    NSString *text = [string isKindOfClass:[NSAttributedString class]]
                         ? [(NSAttributedString *)string string]
                         : (NSString *)string;
    if (![text isKindOfClass:[NSString class]] || text.length == 0) {
        return;
    }
    int32_t rep_from = (replacementRange.location == NSNotFound)
                           ? -1
                           : (int32_t)replacementRange.location;
    int32_t rep_to = (replacementRange.location == NSNotFound)
                         ? -1
                         : (int32_t)(replacementRange.location + replacementRange.length);
    printf("[osr] NSTextInputClient insertText \"%s\" rep={%ld,%lu}\n", text.UTF8String,
           (long)replacementRange.location, (unsigned long)replacementRange.length);
    fflush(stdout);
    _markedText = @"";
    _markedRange = NSMakeRange(NSNotFound, 0);
    if (gImeCallbacks.commit_text) {
        gImeCallbacks.commit_text(text.UTF8String, rep_from, rep_to);
    }
}

- (void)unmarkText {
    _markedText = @"";
    _markedRange = NSMakeRange(NSNotFound, 0);
    printf("[osr] NSTextInputClient unmarkText\n");
    fflush(stdout);
    if (gImeCallbacks.finish_composing) {
        gImeCallbacks.finish_composing(1);
    }
}

- (BOOL)hasMarkedText {
    return _markedRange.location != NSNotFound;
}

- (NSRange)markedRange {
    return _markedRange;
}

/// CEF 只在 composition 变化时回报选区,非 composition 的文档选区(需 on_text_selection_changed)
/// 留到 T5;此处如实回报"未知"。
- (NSRange)selectedRange {
    return _hasCefSelection ? _cefSelectedRange : NSMakeRange(NSNotFound, 0);
}

- (NSAttributedString *)attributedSubstringForProposedRange:(NSRange)range
                                               actualRange:(NSRangePointer)actualRange {
    (void)range;
    (void)actualRange;
    return nil;
}

- (NSArray<NSAttributedStringKey> *)validAttributesForMarkedText {
    // 中日韩输入法靠这些属性给 composition 分词/画下划线;返回空数组时,部分输入法会直接
    // 判定"客户端不支持 composition",把按键原样透传(实测微信输入法如此)。属性集与 CEF
    // 自带的 mac OSR 客户端(tests/cefclient/browser/text_input_client_osr_mac.mm)一致。
    // (CEF 参考实现里还有一个 NSTextInputReplacementRangeAttributeName,SDK 头文件未声明,
    // 这里不带;分词真正依赖的是 NSMarkedClauseSegmentAttributeName。)
    return @[
        NSUnderlineStyleAttributeName, NSUnderlineColorAttributeName,
        NSMarkedClauseSegmentAttributeName
    ];
}

/// 候选框/符号面板的锚点(屏幕坐标)。数据来自 CEF 的
/// on_ime_composition_range_changed(composition 期间逐字的位置)。
- (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actualRange {
    if (actualRange) {
        *actualRange = range;
    }
    // 兜底:没有 composition 几何时锚在视图左上角,而不是屏幕原点。
    NSRect caret = _hasCaretRect ? _caretRectDIP : NSMakeRect(4, 4, 1, 16);
    // DIP(左上原点、y 向下)→ 视图坐标(左下原点、y 向上)
    NSRect viewRect = NSMakeRect(caret.origin.x,
                                 self.bounds.size.height - (caret.origin.y + caret.size.height),
                                 MAX(caret.size.width, 1.0), MAX(caret.size.height, 1.0));
    NSRect inWindow = [self convertRect:viewRect toView:nil];
    NSRect inScreen = self.window ? [self.window convertRectToScreen:inWindow] : inWindow;
    printf("[osr] firstRectForCharacterRange → screen=(%.1f,%.1f) %.1fx%.1f"
           "(cef DIP=(%.1f,%.1f) %.1fx%.1f cached=%d)\n",
           inScreen.origin.x, inScreen.origin.y, inScreen.size.width, inScreen.size.height,
           caret.origin.x, caret.origin.y, caret.size.width, caret.size.height, (int)_hasCaretRect);
    fflush(stdout);
    return inScreen;
}

- (NSUInteger)characterIndexForPoint:(NSPoint)point {
    (void)point;
    return NSNotFound;
}

@end

/// 创建 OSR 宿主视图并加入容器(参数为容器坐标,与 CEF 的 windowed 子视图同口径)。
void *osr_host_view_new(void *container, double x, double y, double w, double h) {
    NSView *parent = (__bridge NSView *)container;
    NSRect frame = NSMakeRect(x, y, w, h);
    NSView *view = [[OsrHostView alloc] initWithFrame:frame];
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

#pragma mark - T2:焦点 / IME 几何 / 自检

/// 让宿主视图成为 first responder(windowless 下没有 CEF 自建视图替我们抢焦点)。
void osr_host_view_focus(void *view) {
    NSView *v = (__bridge NSView *)view;
    if (v == nil) {
        return;
    }
    BOOL ok = [v.window makeFirstResponder:v];
    NSResponder *fr = v.window.firstResponder;
    printf("[osr] host view focus: makeFirstResponder=%d firstResponder=%s isKeyWindow=%d "
           "appActive=%d\n",
           (int)ok, fr ? class_getName([fr class]) : "(nil)", (int)v.window.isKeyWindow,
           (int)NSApp.isActive);
    fflush(stdout);
}

/// 缓存 CEF 回报的 composition 几何(DIP、左上原点)+ 选区,供候选框定位。
void osr_host_view_set_ime_bounds(void *view, int32_t sel_from, int32_t sel_to, double x,
                                  double y, double w, double h, int has_bounds) {
    OsrHostView *v = (__bridge OsrHostView *)view;
    if (v == nil) {
        return;
    }
    v->_hasCefSelection = sel_from >= 0 && sel_to >= sel_from;
    v->_cefSelectedRange =
        v->_hasCefSelection ? NSMakeRange((NSUInteger)sel_from, (NSUInteger)(sel_to - sel_from))
                            : NSMakeRange(NSNotFound, 0);
    v->_hasCaretRect = has_bounds != 0;
    v->_caretRectDIP = has_bounds ? NSMakeRect(x, y, w, h) : NSZeroRect;
}

/// 打印当前输入源(判定"这次按键到底是输入法还是直接键入"的关键上下文)。
void osr_log_input_source(void) {
    TISInputSourceRef source = TISCopyCurrentKeyboardInputSource();
    if (source == NULL) {
        printf("[osr] input source: (none)\n");
        fflush(stdout);
        return;
    }
    char name[256] = {0};
    char identifier[256] = {0};
    CFStringRef name_ref = TISGetInputSourceProperty(source, kTISPropertyLocalizedName);
    CFStringRef id_ref = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
    if (name_ref) {
        CFStringGetCString(name_ref, name, sizeof(name), kCFStringEncodingUTF8);
    }
    if (id_ref) {
        CFStringGetCString(id_ref, identifier, sizeof(identifier), kCFStringEncodingUTF8);
    }
    printf("[osr] input source name=\"%s\" id=\"%s\"\n", name, identifier);
    fflush(stdout);
    CFRelease(source);
}

static void osr_after(double delay, dispatch_block_t block) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(delay * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), block);
}

/// 自检 A(确定性):直接按 NSTextInputClient 契约调用宿主视图的方法,
/// 证明 ObjC → Rust → CEF → 页面 textarea 的整条链路,不依赖真实输入法。
void osr_schedule_ime_selftest(double delay) {
    osr_after(delay, ^{
        OsrHostView *v = (OsrHostView *)gOsrView;
        if (v == nil) {
            return;
        }
        printf("[osr] === IME 自检 A:直接调用 NSTextInputClient ===\n");
        fflush(stdout);
        // 1) composition 更新两次(模拟拼音串 nihao,第二次光标停在中间)
        [v setMarkedText:@"nihao"
            selectedRange:NSMakeRange(5, 0)
         replacementRange:NSMakeRange(NSNotFound, 0)];
        osr_after(0.4, ^{
          [v setMarkedText:@"ni hao"
              selectedRange:NSMakeRange(3, 3)
           replacementRange:NSMakeRange(NSNotFound, 0)];
        });
        // 2) 候选上屏
        osr_after(0.8, ^{
          [v insertText:@"你好" replacementRange:NSMakeRange(NSNotFound, 0)];
          printf("[osr] === IME 自检 A 完成 ===\n");
          fflush(stdout);
        });
    });
}

/// 合成一次 keyDown 并投给窗口(进程内合成,不走辅助功能权限;与鼠标探针同 Path)。
static void osr_synth_key(unsigned short key_code, NSString *chars) {
    NSWindow *window = gOsrView.window;
    if (window == nil) {
        return;
    }
    NSEvent *event = [NSEvent keyEventWithType:NSEventTypeKeyDown
                                      location:NSMakePoint(0, 0)
                                 modifierFlags:0
                                     timestamp:NSProcessInfo.processInfo.systemUptime
                                  windowNumber:window.windowNumber
                                       context:nil
                                    characters:chars
                       charactersIgnoringModifiers:chars
                                     isARepeat:NO
                                       keyCode:key_code];
    printf("[osr] synthKey keyCode=%d chars=\"%s\" isKeyWindow=%d appActive=%d\n", (int)key_code,
           chars.UTF8String, (int)window.isKeyWindow, (int)NSApp.isActive);
    fflush(stdout);
    [window sendEvent:event];
}

/// 自检 B(真实输入法):合成 "nihao" + 空格,经由当前输入源(如微信输入法)走
/// setMarkedText / insertText。输入源不是输入法时,这里只会退化成直接键入。
void osr_schedule_ime_keytest(double delay) {
    osr_after(delay, ^{
        printf("[osr] === IME 自检 B:合成按键(经真实输入法)===\n");
        osr_log_input_source();
        // mac 虚拟键码:n=45 i=34 h=4 a=0 o=31,空格=49
        const unsigned short codes[] = {45, 34, 4, 0, 31, 49};
        NSString *chars = @"nihao ";
        for (NSUInteger i = 0; i < 6; i++) {
            unsigned short code = codes[i];
            NSString *ch = [chars substringWithRange:NSMakeRange(i, 1)];
            osr_after(0.2 * (double)(i + 1), ^{
              osr_synth_key(code, ch);
            });
        }
        osr_after(0.2 * 8, ^{
          printf("[osr] === IME 自检 B 完成 ===\n");
          fflush(stdout);
        });
    });
}
