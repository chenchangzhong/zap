// zap-bridge-client — dsh 浏览器端插件(项目切换通知 Zap + 终态通知 + 文件浏览器按钮)。
//
// 运行位置:dsh Web UI 的 cordis 浏览器端(非宿主端)。
// 加载方式:作为 `dsh.client` 声明的 client 包,由 client-modules serve
// `/plugins/zap-bridge-client/client.js`,浏览器端 `__ModuleLoader__` 加载。
//
// 职责:
// - 订阅 sessions/workspaces 快照,检测"当前活跃会话所属项目(workspace)"切换
// - 首次加载(快照 ready)后,通知 Zap 当前是哪个项目
// - 项目切换时,经 webview IPC 发 `zap.switch_project`,让 Zap 跟随
// - 订阅 sessions 快照,检测会话终态(完成/需确认),经 webview IPC 发 `zap.notify`
// - 为侧边栏每个项目行注入"打开文件浏览器"按钮(hover 显示,右侧第一位)
//
// 通信方式:webview IPC(webkit.messageHandlers.ipc.postMessage)。
// 不再使用 WebSocket 桥(端口每次启动随机分配导致连接不稳定)。
//
// 项目路径判定:当前会话(SessionSummary)的 `cwd` 字段是权威来源(实测存在,
// 如 "/Users/zhong/project/dsh-plugins");`workspaceId` 不在 summary 上,故
// 用 cwd 直接作项目目录,并回退到 workspace 列表按 sessionIds 反查。
//
// 终态判定:基于 `dsh-client-runtime` 提供的 `sessions` 服务快照(dsh-client-runtime
// `SessionManager` 经 `ctx.reflect.provide("sessions")` 注册)。快照形状:
// `{ ids, byId: { [sessionId]: SessionEntry }, current, phase }`,其中
// SessionEntry 含 `running`, `completed`, `pendingInteraction`, `title`, `cwd` 等。
// 终态映射:`pendingInteraction` -> confirm(需确认), `completed` 或
// `running:true -> false` 边沿 -> complete。无专用 `notifications` 服务时以
// `sessions` 状态变迁代理,符合 cordis `inject + ctx.effect` 模型。
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
		/// 终态去重:lastNotifiedKey = sessionId + ":" + status。
		let lastNotifiedKeys = new Map();
		/// 上一次 running 状态(检测 running:true -> false 边沿)。
		let prevRunning = new Map();

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

		/// 上报终态通知给 Zap(经 IPC 发 zap.notify)。
		/// category 仅三值 "complete"|"error"|"confirm",对应 Rust bridge.rs 映射。
		function reportNotify(title, body, category) {
			if (!title) return;
			const cat = category || "complete";
			zapRpc('zap.notify', { title, body: body || "", category: cat }).catch(err => {
				console.error("[zap-bridge-client] notify failed:", err);
			});
			console.log("[zap-bridge-client] notify ->", title, cat);
		}

		/// 从 cwd 取末段作标题回退。
		function cwdBasename(cwd) {
			if (!cwd) return undefined;
			const parts = cwd.replace(/\/+$/, "").split("/");
			return parts[parts.length - 1] || undefined;
		}

		function pendingText(v) {
			if (typeof v === 'string') return v.slice(0, 500);
			try {
				const s = JSON.stringify(v);
				return s ? s.slice(0, 500) : String(v).slice(0, 500);
			} catch {
				return String(v).slice(0, 500);
			}
		}

		/// 检测会话终态并上报(去重:同一 session 同一 status 仅发一次)。
		function checkNotify() {
			if (!sessionsRef) return;
			const snap = sessionsRef.list.getSnapshot();
			if (snap.phase !== "ready") return;
			const byId = snap.byId || {};
			for (const sid of Object.keys(byId)) {
				const entry = byId[sid];
				if (!entry) continue;
				if (entry.blank) {
					prevRunning.set(sid, entry.running);
					continue;
				}
				let status;
				let category;
				if (entry.pendingInteraction) {
					status = "confirm";
					category = "confirm";
				} else if (entry.completed) {
					status = "complete";
					category = "complete";
				} else {
					const prev = prevRunning.get(sid);
					const curr = entry.running;
					if (prev === true && curr === false) {
						status = "complete";
						category = "complete";
					} else {
						prevRunning.set(sid, curr);
						continue;
					}
				}
				prevRunning.set(sid, entry.running);
				const bodyText = entry.pendingInteraction ? pendingText(entry.pendingInteraction) : (entry.title ? "" : (entry.cwd || ""));
				const key = status === "confirm" ? (sid + ":" + status + ":" + bodyText) : (sid + ":" + status);
				if (lastNotifiedKeys.get(sid) === key) continue;
				lastNotifiedKeys.set(sid, key);
				const title = entry.title || cwdBasename(entry.cwd) || "DSH task";
				reportNotify(title, bodyText, category);
			}
			for (const sid of [...prevRunning.keys()]) {
				if (!byId[sid]) {
					prevRunning.delete(sid);
					lastNotifiedKeys.delete(sid);
				}
			}
			for (const sid of [...lastNotifiedKeys.keys()]) {
				if (!byId[sid]) lastNotifiedKeys.delete(sid);
			}
		}

		// ── 文件浏览器按钮:侧边栏每项目行 hover 显示,右侧第一位 ──

		const FILE_EXPLORER_SVG = '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M5.19629 1.57104C5.81144 1.5711 6.38623 1.8786 6.72754 2.39038L7.19922 3.09839C7.28454 3.22635 7.42824 3.30344 7.58203 3.30347H12.1699C13.5039 3.30348 14.5859 4.38548 14.5859 5.71948V6.62671C15.2694 7.02689 15.6605 7.85012 15.4385 8.68726L14.3848 12.658C14.1037 13.7164 13.1449 14.4527 12.0498 14.4529H2.91699C1.51651 14.4529 0.451662 13.2814 0.501954 11.9519V3.98706C0.501954 2.65305 1.58396 1.57104 2.91797 1.57104H5.19629ZM3.7793 7.75562C3.30994 7.75562 2.89883 8.07153 2.77832 8.52515L1.91602 11.7722C1.74167 12.4291 2.23734 13.073 2.91699 13.073H12.0498C12.5191 13.0728 12.9304 12.757 13.0508 12.3035L14.1045 8.33374C14.1819 8.04202 13.9619 7.756 13.6602 7.75562H3.7793ZM2.91797 2.9519C2.34625 2.9519 1.88281 3.41534 1.88281 3.98706V7.2937C2.33068 6.7269 3.02249 6.37476 3.7793 6.37476H13.2051V5.71948C13.2051 5.14777 12.7416 4.68434 12.1699 4.68433H7.58203C6.96675 4.6843 6.39209 4.37595 6.05078 3.86401L5.5791 3.15601C5.49379 3.02821 5.34995 2.95196 5.19629 2.9519H2.91797Z" fill="currentColor"/></svg>';

		function syncFileExplorerButtons() {
			if (!workspacesRef) return;
			const snap = workspacesRef.list.getSnapshot();
			if (snap.phase !== "ready") return;
			const items = snap.items || [];
			if (items.length === 0) return;
			const rows = document.querySelectorAll('[class*="projectRow"]');
			if (rows.length === 0) return;
			const labelToWs = new Map();
			for (const w of items) {
				if (!w.path) continue;
				const label = w.title || w.path.replace(/\/+$/, "").split("/").pop() || w.path;
				if (!labelToWs.has(label)) labelToWs.set(label, []);
				labelToWs.get(label).push(w);
			}
			const usedWsPaths = new Set();
			rows.forEach((row) => {
				const actions = row.querySelector('[class*="rowActions"]');
				if (!actions) return;
				const rowText = (row.textContent || "").trim();
				if (rowText === "" || rowText.startsWith("Ungrouped")) return;
				// 优先用标题元素的精确文本,避免 includes 子串把 my-app 命中 app
				const titleEl = row.querySelector('[class*="title"]') || row.querySelector('[class*="projectText"]');
				const titleText = titleEl ? (titleEl.textContent || "").trim() : "";
				let matched = null;
				if (titleText && labelToWs.has(titleText)) {
					const list = labelToWs.get(titleText);
					matched = list.find(w => !usedWsPaths.has(w.path)) || null;
				}
				if (!matched) {
					let bestMatch = null;
					let bestLen = -1;
					for (const [label, list] of labelToWs.entries()) {
						if (!rowText.includes(label)) continue;
						const unused = list.find(w => !usedWsPaths.has(w.path));
						if (!unused) continue;
						if (label.length > bestLen) { bestLen = label.length; bestMatch = unused; }
					}
					matched = bestMatch;
				}
				if (!matched) return;
				usedWsPaths.add(matched.path);
				const matchedPath = matched.path;
				const existing = row.querySelector('[data-zap-file-explorer]');
				if (existing) {
					if (existing.getAttribute('data-zap-file-explorer') !== matchedPath) {
						existing.setAttribute('data-zap-file-explorer', matchedPath);
					}
					return;
				}
				const btn = document.createElement("button");
				btn.type = "button";
				btn.setAttribute("data-zap-file-explorer", matchedPath);
				btn.setAttribute("aria-label", "打开文件浏览器");
				btn.setAttribute("title", "打开文件浏览器");
				const refCls = actions.querySelector('[class*="iconButton"]')?.className || "";
				btn.className = refCls;
				if (!refCls) {
					btn.style.width = "20px";
					btn.style.height = "20px";
					btn.style.border = "none";
					btn.style.background = "transparent";
					btn.style.cursor = "pointer";
					btn.style.color = "var(--dsw-alias-label-tertiary)";
				}
				btn.style.display = "inline-flex";
				btn.style.alignItems = "center";
				btn.style.justifyContent = "center";
				btn.innerHTML = FILE_EXPLORER_SVG;
				btn.addEventListener("click", (e) => {
					e.stopPropagation();
					e.preventDefault();
					const p = btn.getAttribute('data-zap-file-explorer') || matchedPath;
					zapRpc('zap.open_file_explorer', { path: p }).catch(err => {
						console.error("[zap-bridge-client] open_file_explorer failed:", err);
					});
					console.log("[zap-bridge-client] open_file_explorer ->", p);
				});
				actions.prepend(btn);
			});
		}

		let pendingSyncRaf = 0;
		function scheduleSyncFileExplorerButtons() {
			if (pendingSyncRaf) return;
			pendingSyncRaf = requestAnimationFrame(() => {
				pendingSyncRaf = 0;
				syncFileExplorerButtons();
			});
		}

		function apply(ctx) {
			// 非 Zap 环境:不注册任何能力,直接退出。
			if (!window.__ZAP_BRIDGE__) {
				return;
			}
			const sessions = ctx.get("sessions");
			const workspaces = ctx.get("workspaces");
			if (!sessions || !workspaces) {
				console.error("[zap-bridge-client] sessions/workspaces unavailable");
				return;
			}
			sessionsRef = sessions;
			workspacesRef = workspaces;

			const unsubSessions = sessions.list.subscribe(reportCurrentPath);
			const unsubWorkspaces = workspaces.list.subscribe(reportCurrentPath);
			const unsubFileExplorer = workspaces.list.subscribe(scheduleSyncFileExplorerButtons);
			const unsubNotify = sessions.list.subscribe(checkNotify);
			reportCurrentPath();
			checkNotify();
			scheduleSyncFileExplorerButtons();
			const observer = new MutationObserver(() => scheduleSyncFileExplorerButtons());
			observer.observe(document.body, { childList: true, subtree: true });

			ctx.effect(() => {
				return () => {
					unsubSessions();
					unsubWorkspaces();
					unsubFileExplorer();
					unsubNotify();
					if (pendingSyncRaf) cancelAnimationFrame(pendingSyncRaf);
					observer.disconnect();
					document.querySelectorAll('[data-zap-file-explorer]').forEach(el => el.remove());
					delete window.__onZapResponse;
				};
			});
		}

		exports.apply = apply;
		exports.inject = inject;
		return module.exports;
	}
});
