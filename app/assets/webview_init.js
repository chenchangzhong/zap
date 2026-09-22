
// ---- 文档开始前的引导(CEF 在 render 进程于 OnContextCreated 注入;wry 用 WKUserScript)----
// CEF 侧带 token 的真实 shim 在 load_end 注入(它必须由 browser 进程生成);在那之前
// 任何 postMessage 先入队,待真实 shim 就位后回放,避免"启动期的 zapRpc 永久丢失"。
// 【临时诊断】自证"文档开始前注入"生效:此刻真实 shim 可能还没到,消息会入队,
// 待 shim 就位后回放 → 服务端日志出现 "[dsh-loopback] webview-init-ready"。
window.webkit = window.webkit || {};
window.webkit.messageHandlers = window.webkit.messageHandlers || {};
if (!window.webkit.messageHandlers.ipc) {
  const __zapQueue = [];
  window.__zapIpcQueue = __zapQueue;
  window.webkit.messageHandlers.ipc = { postMessage: (message) => __zapQueue.push(message) };
}

window.__ZAP_BRIDGE__ = true;
// JS 错误/警告转发到 Rust 日志(诊断用)。
window.addEventListener('error', (e) => {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:' + (e.message || 'unknown'));
});
window.addEventListener('unhandledrejection', (e) => {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:unhandledrejection:' + String(e.reason).slice(0, 200));
});
const __origLog = console.error;
console.error = function(...args) {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:console:' + args.map(String).join(' ').slice(0, 300));
  __origLog.apply(console, args);
};

document.addEventListener('keydown', (e) => {
  if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  // Cmd+R → 页面刷新(wry child webview 的 performKeyEquivalent 返回 NO,
  // 不触发 KVO 刷新,需 JS 手动处理)。
  if (e.key === 'r' || e.key === 'R') {
    e.preventDefault();
    location.reload();
  }
});
document.addEventListener('focusin', () => {
  if (document.activeElement && document.activeElement !== document.body) {
    // 页面已持有文档焦点时不再上报:此时上报只会触发重复的 makeFirstResponder,
    // 而 WebKit 在该过程中会把当前聚焦元素 blur 到 body(relatedTarget=null),
    // 模型菜单等弹层 onBlur 即被关闭,点击落空(表现为切换模型失败)。
    if (document.hasFocus()) return;
    window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-focusin');
  }
});
// 方向键修字符插入:DSH 输入框是 contenteditable div(DIV.uV2eYG_input,
// contenteditable 由容器继承)。实测按方向键会经 WebKit 编辑兜底路径向光标
// 处插入 U+001D(Group Separator,渲染为方块、复制后不可见),且不触发
// beforeinput/input;与输入法无关,普通浏览器无此问题(Chromium 不走该
// 兜底)。此处对 contenteditable 内的方向键 preventDefault 并用
// Selection.modify()(WebKit 扩展 API)移动光标,完全绕开 WebKit 的字符插入路径。
document.addEventListener('keydown', function(e) {
  if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
  if (e.isComposing) return;
  if (e.defaultPrevented) return;
  // 修饰键组合(Cmd+← 行首/尾、Option+← 词跳、Shift+← 扩选)交回原生路径:
  // 兜底插入只在无修饰的裸方向键上出现,且单字符 move 会丢失词跳/扩选语义。
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  var ae = document.activeElement;
  if (!ae || ae.tagName !== 'DIV' || !ae.isContentEditable) return;
  e.preventDefault();
  var sel = window.getSelection();
  if (sel && sel.anchorNode) {
    sel.modify('move', e.key === 'ArrowRight' ? 'forward' : 'backward', 'character');
  }
}, true);
// 点击页面任意位置上报 Rust,让 WKWebView 同步成为 first responder。
// 仅在页面尚未持有文档焦点时上报(首次从 Warp 侧点进页面);页面已持焦时
// 重复的 makeFirstResponder 会让 WebKit 把当前聚焦元素 blur 到 body
// (relatedTarget=null),弹层 onBlur 即关、点击落空。
document.addEventListener('mousedown', () => {
  if (document.hasFocus()) return;
  window.focus();
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-mousedown');
});
// 点击链接:一律用系统默认浏览器打开,不在 webview 内导航。
// 锚点(#...)与 javascript: 伪协议链接不拦截。兼容 HTML 与 SVG <a>。
document.addEventListener('click', (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  let el = e.target;
  while (el && el.tagName !== 'A') el = el.parentElement;
  if (!el || el.tagName !== 'A') return;
  const rawHref = el.getAttribute('href');
  if (!rawHref || rawHref.startsWith('#') || rawHref.startsWith('javascript:')) return;
  e.preventDefault();
  // HTML <a> 的 href 是字符串;SVG <a> 的是 SVGAnimatedString,需用
  // baseURI 重新解析。解析失败(非法 URL)则吞掉点击,不导航不外部打开。
  let target;
  try {
    target = typeof el.href === 'string' ? el.href : new URL(rawHref, document.baseURI).href;
  } catch {
    return;
  }
  // <a download>(如 dsh Session 日志导出的 JS 合成下载链接):不拦。
  // WKWebView 的原生下载管道(WKDownloadDelegate,wry 已接 download
  // handler)会继承页面的会话 cookie 完成下载(Electron/Chromium 同款,
  // 默认 ~/Downloads,Zap 侧另弹保存面板);若在此 preventDefault 转系统
  // 浏览器,裸 URL 缺 dsh 的 cookie(登录 token 只在 GET / 种 cookie)
  // 只会拿到 401。
  if (el.hasAttribute('download')) return;
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:open-external:' + target);
});
// window.open():同样交给系统默认浏览器,不创建新窗口也不在当前 webview 导航。
window.open = function(url) {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:open-external:' + url);
  return null;
};
// 切回(webview 重新获得键盘焦点)时把输入框聚焦,并把光标折叠到内容末尾。
// dsh 页面只有一个输入框,直接按选择器找,不做失焦记录;无论 DOM 焦点是否
// 一直残留在输入框,光标都统一到末尾。
window.__restoreFocused = function() {
  // 两个独立计数:元素缺失(SPA 首渲染晚于 load,需要 ~3s 窗口)与页面未就绪
  // (makeFirstResponder 后 hasFocus 要等 AppKit 事件循环,原实现是 ~600ms)。
  var missingAttempts = 0;
  var focusAttempts = 0;
  var tryFocus = function() {
    var el = document.querySelector('textarea[data-testid="dsh-input"]')
             || document.querySelector('textarea[placeholder]')
             || document.querySelector('div[contenteditable="true"][role="textbox"]')
             || document.querySelector('div.ProseMirror')
             || document.querySelector('.cm-content[contenteditable]');
    // 元素还没挂载时必须**重试**:load 事件早于 SPA 首次渲染,此刻输入框往往
    // 还不在 DOM 里(CEF pane 实测:加载完成时 document.activeElement 是 BODY、
    // 找不到输入框),原来直接 return 会让"打开 pane 后不点页面直接打字"完全
    // 没有反应 —— 焦点永远留在 body。有限次(≈3s)避免死循环。
    if (!el || !el.isConnected) {
      if (missingAttempts++ < 60) {
        setTimeout(tryFocus, 50);
      }
      return;
    }
    if (document.hasFocus()) {
      el.focus();
      // 光标一律折叠到内容末尾:不做失焦 blur 后 DOM 焦点常驻输入框,WebKit
      // 原生保留的 caret 位置不可控(可能停在开头/任意处),按约定统一到
      // 末尾:切回后用户要接着输入。
      if (typeof el.setSelectionRange === 'function') {
        try { el.setSelectionRange(el.value.length, el.value.length); } catch (e) {}
      } else if (el.isContentEditable) {
        try {
          var endRange = document.createRange();
          endRange.selectNodeContents(el);
          endRange.collapse(false);
          var endSel = window.getSelection();
          endSel.removeAllRanges();
          endSel.addRange(endRange);
        } catch (e) {}
      }
    } else if (focusAttempts++ < 20) {
      // makeFirstResponder 后页面 hasFocus 需等 AppKit 事件循环才变 true,故重试等待,
      // 有限次避免死循环。**窗口保持原来的 ~600ms**:拉长到 3s 会把"聚焦 + 光标折叠到末尾"
      // 推迟太久,期间用户自己移动光标/选择会被这条链覆盖(且该脚本 wry 路径共用)。
      setTimeout(tryFocus, 30);
    }
  };
  tryFocus();
};
// WebKit 兼容:菜单内 mousedown 的默认动作会把当前聚焦元素 blur 到 body
// (relatedTarget=null;Chromium 同场景是把焦点移入被点的按钮,故浏览器无此
// 问题)。官方模型菜单的 onBlur 见到焦点落到 body 即收起菜单,导致「焦点已
// 在菜单项上」的第二次点击必然先触发 focusout → 菜单卸载 → click 落空
// (表现为切换模型失败)。对菜单内 mousedown preventDefault 阻止该默认
// blur;click 照常派发,菜单项选择不受影响。
document.addEventListener('mousedown', function (e) {
  if (!e.target || !e.target.closest || !e.target.closest('[role="menu"]')) return;
  e.preventDefault();
}, true);

// 【临时诊断】握手:证明引导脚本确实在页面里执行了。
window.webkit.messageHandlers.ipc.postMessage('warp:webview-init-ready');
