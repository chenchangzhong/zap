// 无 GUI 取证:加载 CEF framework 后,用 ObjC runtime 直接查询 Chromium 宿主视图
// 是否实现标准编辑 selector(评审 Important 4 的"未验证推测")。
#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#import <dlfcn.h>
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc < 2) { printf("usage: probe <framework-binary>\n"); return 2; }
    void *handle = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!handle) { printf("dlopen 失败: %s\n", dlerror()); return 1; }

    const char *classNames[] = {"RenderWidgetHostViewCocoa", "CefBrowserHostView", NULL};
    const char *selectors[] = {"paste:", "cut:", "selectAll:", "copy:", NULL};
    for (int i = 0; classNames[i]; i++) {
        Class cls = objc_getClass(classNames[i]);
        printf("class %s: %s\n", classNames[i], cls ? "found" : "NOT found");
        if (!cls) continue;
        for (int j = 0; selectors[j]; j++) {
            BOOL responds = class_respondsToSelector(cls, sel_registerName(selectors[j]));
            printf("  %-11s %s\n", selectors[j], responds ? "YES" : "no");
        }
    }
    return 0;
}
