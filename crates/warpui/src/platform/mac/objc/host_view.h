#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

@interface NSPasteboard (Warp)
- (NSArray *)getFilePaths;
@end

/// The view that hosts the CAMetalLayer backing for the Warp renderer. Sits
/// above WebViewContainerView so its content covers any embedded webview;
/// hitTest routes events into WarpHostView over overlay UI regions and lets
/// them pass through elsewhere.
@interface MetalRenderView : NSView
- (instancetype)initWithFrame:(NSRect)frame metalDevice:(id)metalDevice;
@end

/// Transparent container below MetalRenderView that hosts the embedded webview
/// (WKWebView via wry). No behavior of its own.
@interface WebViewContainerView : NSView
@end

/// A CAMetalLayer-backed view that is the full-window background surface, kept
/// as the first subview of WebViewContainerView so it sits below any embedded
/// webview. The render loop clears it (full-window) with the workspace
/// background color; a transparent webview above it shows this same color
/// instead of the desktop. Because it is a Metal layer (like the main renderer)
/// the compositing math matches the Metal-drawn UI exactly.
@interface MetalBackgroundView : NSView
- (instancetype)initWithFrame:(NSRect)frame metalDevice:(id)metalDevice;
@end

/// WarpHostView is the Content view of a Warp window.
// It is backed by a plain (transparent) CALayer; actual Metal rendering happens
// in MetalRenderView, which sits above WebViewContainerView in the view
// hierarchy so that overlay UI draws on top of any embedded webview.
@interface WarpHostView : NSView <CALayerDelegate, NSTextInputClient>
- (WarpHostView *)initWithFrame:(NSRect)frame
                    metalDevice:(id)metalDevice
             enableTitlebarDrag:(BOOL)enableTitlebarDrag
                       testMode:(BOOL)testMode;
- (void)setAsyncCallback:(BOOL)shouldAsync;
- (void)setPresentsWithTransaction:(BOOL)presentsWithTransaction;
- (BOOL)keyDownImpl:(NSEvent *)event;
@property (nonatomic, retain, readwrite) MetalRenderView *metalRenderView;
@property (nonatomic, retain, readwrite) WebViewContainerView *webViewContainer;
@property (nonatomic, retain, readwrite) MetalBackgroundView *metalBackgroundView;
@end

