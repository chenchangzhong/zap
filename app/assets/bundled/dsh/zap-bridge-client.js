// zap-bridge-client — dsh 浏览器端插件(项目切换通知 Zap)。
//
// 运行位置:dsh Web UI 的 cordis 浏览器端(非宿主端)。
// 加载方式:作为 `dsh.client` 声明的 client 包,由 client-modules serve
// `/plugins/zap-bridge-client/client.js`,浏览器端 `__ModuleLoader__` 加载。
//
// 职责:
// - 订阅 sessions/workspaces 快照,检测"当前活跃会话所属项目(workspace)"切换
// - 首次加载(快照 ready)后,通知 Zap 当前是哪个项目
// - 项目切换时,经 webview IPC 发 `zap.switch_project`,让 Zap 跟随
//
// 通信方式:webview IPC(webkit.messageHandlers.ipc.postMessage)。
// 不再使用 WebSocket 桥(端口每次启动随机分配导致连接不稳定)。
//
// 项目路径判定:当前会话(SessionSummary)的 `cwd` 字段是权威来源(实测存在,
// 如 "/Users/zhong/project/dsh-plugins");`workspaceId` 不在 summary 上,故
// 用 cwd 直接作项目目录,并回退到 workspace 列表按 sessionIds 反查。
window.__ModuleLoader__.load({
	id: "@zap/zap-bridge-client",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;
		Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });

		// 依赖 sessions/workspaces 服务(dsh-client-runtime 用 reflect.provide
		// 注册;框架保证 apply 时已就绪)。
		const inject = ["sessions", "workspaces"];

		/// 最近上报的 workspace path(去重:同 path 不重复上报)。
		let lastReportedPath = undefined;
		/// 待响应的 RPC 请求(id → {resolve, reject})。
		const pendingRequests = new Map();
		/// 递增请求 id。
		let nextId = 0;
		/// 供上报的 sessions/workspaces 引用(apply 时赋值)。
		let sessionsRef = undefined;
		let workspacesRef = undefined;

		/// 发送 RPC 请求到 Zap(经 webview IPC)。
		/// 返回 Promise,resolve 时携带 result。
		function zapRpc(method, params) {
			const id = ++nextId;
			const payload = method + '\n' + id + '\n' + JSON.stringify(params || {});
			window.webkit.messageHandlers.ipc.postMessage('zap:' + payload);
			return new Promise((resolve, reject) => {
				pendingRequests.set(id, { resolve, reject });
			});
		}

		/// 接收 Zap 响应(由 Rust evaluate_script 调用)。
		window.__onZapResponse = function(id, result) {
			const pending = pendingRequests.get(id);
			if (pending) {
				pendingRequests.delete(id);
				if (result && result.error) {
					pending.reject(result.error);
				} else {
					pending.resolve(result && result.result);
				}
			}
		};

		/// 计算当前活跃会话所属项目的绝对路径。
		///
		/// 优先用会话 `cwd`(SessionSummary 实测字段,即项目目录);缺失时回退
		/// 到 workspace 列表按 `sessionIds` 反查(workspace.path 为项目根)。
		function currentWorkspacePath(sessions, workspaces) {
			const sessionSnap = sessions.list.getSnapshot();
			const currentId = sessionSnap.current;
			if (!currentId) return undefined;
			const session = sessionSnap.byId[currentId];
			if (!session) return undefined;
			if (session.cwd) return session.cwd;
			// 回退:workspace.sessionIds 含当前会话 → 该项目根。
			const workspace = workspaces.list
				.getSnapshot()
				.items.find((w) => w.sessionIds && w.sessionIds.includes(currentId));
			return workspace?.path;
		}

		/// 读取当前快照并上报(首载 / 切换时主动调用)。
		function reportCurrentPath() {
			if (!sessionsRef || !workspacesRef) return;
			const sessionsPhase = sessionsRef.list.getSnapshot().phase;
			const workspacesPhase = workspacesRef.list.getSnapshot().phase;
			if (sessionsPhase !== "ready" || workspacesPhase !== "ready") return;
			const path = currentWorkspacePath(sessionsRef, workspacesRef);
			if (path) reportPath(path);
		}

		/// 上报一个 path 给 Zap(经 IPC 发 zap.switch_project)。
		function reportPath(path) {
			if (!path || path === lastReportedPath) return;
			lastReportedPath = path;
			zapRpc('zap.switch_project', { path }).catch(err => {
				console.error("[zap-bridge-client] switch_project failed:", err);
			});
			console.log("[zap-bridge-client] switch_project ->", path);
		}

		function apply(ctx) {
			const sessions = ctx.get("sessions");
			const workspaces = ctx.get("workspaces");
			if (!sessions || !workspaces) {
				console.error("[zap-bridge-client] sessions/workspaces unavailable");
				return;
			}
			sessionsRef = sessions;
			workspacesRef = workspaces;

			// 订阅 sessions(会话选择变化)与 workspaces(工作区映射变化)。
			// 变化时读取当前 path 并上报(reportPath 内部按 path 去重)。
			const unsubSessions = sessions.list.subscribe(reportCurrentPath);
			const unsubWorkspaces = workspaces.list.subscribe(reportCurrentPath);
			// 首次检查(快照可能已 ready)→ 首载通知当前项目。
			reportCurrentPath();

			ctx.effect(() => {
				return () => {
					unsubSessions();
					unsubWorkspaces();
					delete window.__onZapResponse;
				};
			});
		}

		exports.apply = apply;
		exports.inject = inject;
		return module.exports;
	}
});
