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
// - 暴露 `window.__zapInsertFileReference`:「附加为上下文」末端,把文件路径以 dsh `@`
//   引用芯片(ReferenceChipNode)形态插入当前会话输入框(Rust 侧 evaluate_script 调用)
// - 暴露 `window.__zapActivateSession`:通知点击后切到指定会话
//   (统一调公开的 uiWorkspace.openSession;该服务三个版本都提供,≤0.1.6-alpha.1
//   内部委托给 sessions.open)
//
// 通信方式:webview IPC(webkit.messageHandlers.ipc.postMessage)。
// 不再使用 WebSocket 桥(端口每次启动随机分配导致连接不稳定)。
//
// 项目路径判定:当前会话(SessionSummary)的 `cwd` 字段是权威来源(实测存在,
// 如 "/Users/zhong/project/dsh-plugins");`workspaceId` 不在 summary 上,故
// 用 cwd 直接作项目目录,并回退到 workspace 列表按 sessionIds 反查。
// 「当前会话」的来源随 dsh 版本变化:dsh 0.1.6-alpha.2 起 sessions 快照不再带
// `current`(见 dsh-api-session-controller 的 SessionListValue/SessionSummary
// 契约,快照只有 `{ ids, byId, phase, subagentsByParent, jobsBySession }`),
// "当前主会话"改由 `SessionSummary.retainedBy.mainView` 标记。故先读 `current`
// (≤0.1.6-alpha.1),未命中再按 `retainedBy.mainView > 0` 找主会话。
//
// 终态判定:会话 UI 状态(`running` / `pendingInteraction` / `completionUnread`)取自公开的
// `uiSession.sessionStatus`(0.1.6-alpha.2+;旧版两个版本回退公开的
// `uiSession.pendingInteractions` 与 sessions 快照字段),`sessions` 快照另提供
// `title`/`cwd`/`current`/`retainedBy` 等(见 checkNotify 的注记)。
// 终态映射:`pendingInteraction` -> confirm(需确认);complete 由 `running:true -> false`
// 边沿触发(`completionUnread` 仅在观测不到 running 时兜底,旧版则为 `completed`)。
// 无专用 `notifications` 服务时以会话状态变迁代理,符合 cordis `inject + ctx.effect` 模型。
window.__ModuleLoader__.load({
	id: "@zap/zap-bridge-client",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;
		Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });

		// 侧边栏分割线(dsh-client-ui-layout 的 .sidebarCol border-right,
		// 0.5px solid var(--dsw-alias-border-l3))颜色对齐 Zap 左侧边栏
		// outline() 的取值,随 dsh 界面亮暗两套:
		// - 暗色 = Zap 内置 VS Code 2026 Dark 主题 ui.border = #333536(不透明);
		// - 亮色 = Zap 内置 Light 主题无 ui.border,回退 fg_overlay_2 =
		//   foreground #111111 @ 10%(ColorU alpha = 25/255 ≈ 9.8%)。
		// 直接注入 <style>,加载即生效,不依赖 cordis apply 的服务注入。
		(function () {
			if (typeof document === "undefined" || !document.head) return;
			if (document.querySelector('style[data-zap-bridge-css="sidebar-divider"]')) return;
			var style = document.createElement("style");
			style.dataset.zapBridgeCss = "sidebar-divider";
			style.textContent = '[class*="sidebarCol"]{border-right-color:rgb(17 17 17 / 9.8%)}' +
				'body[data-ds-dark-theme] [class*="sidebarCol"]{border-right-color:#333536}' +
				// 聊天页头部(dsh-client-ui-conversation ConversationRoot 的
				// .wSkVaW_header)border-bottom 亦为 .5px l3,同套取值;
				// "_header" 前缀是构建 hash,故用子串匹配(与 sidebarCol 同法)。
				'[class*="_header"]{border-bottom-color:rgb(17 17 17 / 9.8%)}' +
				'body[data-ds-dark-theme] [class*="_header"]{border-bottom-color:#333536}';
			document.head.appendChild(style);
		})();

		// 依赖 sessions/workspaces 服务(dsh-client-runtime 用 reflect.provide
		// 注册;框架保证 apply 时已就绪)。sidebarRight 是 dsh-client-ui-sidebar-right
		// 提供的右侧栏导航 face——dsh 0.1.5 起聊天内文件入口全部收敛到它的
		// openResource,文件链接拦截器 patch 的即是该方法;未声明 inject 直接访问会
		// 报 "cannot get property ... without inject" 并使整个 apply 失败。
		const inject = ["sessions", "workspaces", "sidebarRight"];

		/// 最近上报的 workspace path(去重:同 path 不重复上报)。
		let lastReportedPath = undefined;
		/// 待响应的 RPC 请求(id → {resolve, reject})。
		const pendingRequests = new Map();
		/// 递增请求 id。
		let nextId = 0;
		/// 供上报的 sessions/workspaces 引用(apply 时赋值)。
		let sessionsRef = undefined;
		let workspacesRef = undefined;
		/// apply 期保存的插件上下文:供只在运行期才需要解析的服务(uiSession)
		/// 惰性查找,避免 apply 期的装载顺序依赖。
		let bridgeCtx = undefined;
		/// 上一次 running 状态(检测 running:true -> false 边沿)。
		let prevRunning = new Map();
		/// 上一次 completed 状态(检测 completed:false -> true 边沿;undefined=未观察)。
		let lastCompleted = new Map();
		/// 上一次「后台完成未读」状态(0.1.6-alpha.2+,检测 false -> true 边沿)。
		let lastUnread = new Map();
		/// uiSession 状态订阅的注销函数(惰性建立,见 ensureStatusSubscription)。
		let statusUnsub = undefined;
		/// confirm 去重:同一会话同一文案仅提示一次。
		let lastNotifiedKeys = new Map();

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

		/// 当前主会话 id。
		///
		/// 来源随 dsh 版本变化:
		/// - ≤0.1.6-alpha.1:sessions 快照自带 `current`(实测 0.1.5-alpha.2 的
		///   list 快照仍带 `current`/`currentAddress`);
		/// - 0.1.6-alpha.2 起 `current` 被移除(快照只剩 `{ ids, byId, phase,
		///   subagentsByParent, jobsBySession }`),"当前主会话"改由
		///   `SessionSummary.retainedBy.mainView` 标记——replaceMain 以
		///   `retain(target, { source: "mainView" })` 写入,读取用
		///   `(s.retainedBy.mainView ?? 0) > 0`,这是 dsh 自身十余处
		///   (ui-layout / ui-cordis / ui-session / ui-agent-preset 等)的公开惯用法。
		///
		/// 注:`uiWorkspace.selection` 虽同源,但在 dsh 公开契约里是 private
		/// 字段(navigation.d.ts `private readonly selection`),dsh 自身零消费,
		/// 故不用它,以免耦合非公开面。
		function currentSessionId(sessionSnap) {
			if (sessionSnap.current) return sessionSnap.current;
			const byId = sessionSnap.byId || {};
			return Object.values(byId).find((s) => (s?.retainedBy?.mainView ?? 0) > 0)?.id;
		}

		/// 计算当前活跃会话所属项目的绝对路径。
		///
		/// 优先用会话 `cwd`(SessionSummary 实测字段,即项目目录);缺失时回退
		/// 到 workspace 列表按 `sessionIds` 反查(workspace.path 为项目根)。
		function currentWorkspacePath(sessions, workspaces) {
			const sessionSnap = sessions.list.getSnapshot();
			const currentId = currentSessionId(sessionSnap);
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
		/// sessionId 用于点击通知后在 dsh 内切到对应会话(bridge 透传 →
		/// NotificationItem → __zapActivateSession)。
		function reportNotify(title, body, category, sessionId) {
			if (!title) return;
			const cat = category || "complete";
			const params = { title, body: body || "", category: cat };
			if (sessionId) params.session_id = sessionId;
			zapRpc('zap.notify', params).catch(err => {
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

		/// 把待确认交互渲染成可读文案。
		///
		/// `SessionPendingInteraction` 是各 domain 自有形状的联合(approval 带
		/// toolName、question 带 question 文本……),插件不硬编码具体 domain,只按
		/// 常见字段兜底;直接 stringify 整个对象会让人看不懂通知正文。
		function confirmText(interaction) {
			if (typeof interaction === 'string') return interaction.slice(0, 500);
			if (!interaction || typeof interaction !== 'object') return String(interaction).slice(0, 500);
			// question 域的提问原文在 questions[0].question(PendingQuestion 契约),
			// 该域的交互对象本身不含 reason/prompt 这类字段。
			const firstQuestion = Array.isArray(interaction.questions) && interaction.questions[0];
			const readable = interaction.question || interaction.prompt || interaction.message || interaction.reason
				|| (firstQuestion && (firstQuestion.question || firstQuestion.title));
			if (typeof readable === 'string' && readable) return readable.slice(0, 500);
			const kind = typeof interaction.kind === 'string' ? interaction.kind : 'interaction';
			const tool = typeof interaction.toolName === 'string' ? interaction.toolName : undefined;
			return tool ? `需要确认：${kind} · ${tool}` : `需要确认：${kind}`;
		}

		/// confirm 去重键:优先用 domain 提供的稳定 key,缺失时退回文案本身。
		function confirmKey(interaction) {
			if (interaction && typeof interaction === 'object' && typeof interaction.key === 'string') {
				return interaction.key;
			}
			return confirmText(interaction);
		}

		/// `uiSession.sessionStatus`(公开 readonly 的 HostObservable)。
		///
		/// 它是「running / pendingInteraction / completionUnread」三种会话事实的
		/// 唯一来源:0.1.6-alpha.2 起 SessionSummary 上既没有 completed 也没有
		/// pendingInteraction。更早的版本没有该字段(0.1.5-alpha.2 与
		/// 0.1.6-alpha.1 均无)→ 返回 undefined,调用方走下面的旧版回退。
		function sessionStatusSource() {
			const uiSession = bridgeCtx && bridgeCtx.get("uiSession");
			return uiSession && uiSession.sessionStatus;
		}

		/// 当前会话 UI 状态快照:`Map<SessionId, {running, pendingInteraction,
		/// completionUnread}>`;来源不可用时返回 undefined。
		function sessionStatusSnapshot() {
			const src = sessionStatusSource();
			const map = src && typeof src.getSnapshot === "function" ? src.getSnapshot() : undefined;
			return map && typeof map.get === "function" ? map : undefined;
		}

		/// 旧版回退(两个 0.1.6 之前的版本):UiSession 没有 sessionStatus,但有公开的
		/// `pendingInteractions`(ReadonlyMap<SessionId, SessionPendingInteraction>)。
		/// 只在 sessionStatus 不可用时查询,新版零开销。
		function legacyPendingInteractionMap() {
			const uiSession = bridgeCtx && bridgeCtx.get("uiSession");
			const src = uiSession && uiSession.pendingInteractions;
			const map = src && typeof src.getSnapshot === "function" ? src.getSnapshot() : undefined;
			return map && typeof map.get === "function" ? map : undefined;
		}

		/// 惰性建立 uiSession 状态订阅。
		///
		/// 「需确认」的发布只走 uiSession 的 status notifier(approval/request →
		/// registerPendingInteraction → publishStatus),**不会**把 sessions list
		/// 弄脏——只订阅 list 会整个漏掉它。服务尚未就绪时不建立,留到下次调用重试,
		/// 从而不依赖 apply 期的装载顺序。
		function ensureStatusSubscription() {
			if (statusUnsub) return;
			const uiSession = bridgeCtx && bridgeCtx.get("uiSession");
			const src = uiSession && (uiSession.sessionStatus || uiSession.pendingInteractions);
			if (src && typeof src.subscribe === "function") {
				statusUnsub = src.subscribe(checkNotify);
			}
		}

		/// 检测会话终态并上报。
		/// confirm:同一会话的同一待确认请求只发一次(按 domain 的 key 去重)。
		/// complete:按 turn 触发——running true→false 边沿、completed false→true
		/// 边沿、或首次观察到已完成(兜底补发,每会话一次)。旧逻辑按
		/// sessionId+status 去重,同一会话第二次完成任务会被永久吞掉,已废弃。
		///
		/// 状态来源:`running` 优先取 uiSession.sessionStatus,回退快照字段;
		/// `pendingInteraction` 只有新版 uiSession.sessionStatus 上有(旧版两个
		/// 版本走公开的 uiSession.pendingInteractions),三个版本的 SessionSummary
		/// 都没有它;`completed` 只有旧版快照才有;`completionUnread` 是 dsh 对
		/// 「非主视图会话停止运行、尚待确认」的官方标记,仅在观测不到 running
		/// 边沿时兜底(例如页面刚加载时就已完成)。
		function checkNotify() {
			if (!sessionsRef) return;
			const snap = sessionsRef.list.getSnapshot();
			if (snap.phase !== "ready") return;
			// 「需确认」只改 uiSession 状态、不脏 sessions list,故每次调用都补试
			// 建立订阅(服务未就绪时上轮可能没建成)。
			ensureStatusSubscription();
			const byId = snap.byId || {};
			const statusMap = sessionStatusSnapshot();
			// 旧版(两个 0.1.6 之前的版本)没有 sessionStatus,用公开的
			// pendingInteractions 补「需确认」;running/completed 仍取快照字段。
			const legacyPending = statusMap ? undefined : legacyPendingInteractionMap();
			// uiSession 的覆盖面更广(含已不在列表里的运行中会话),取并集。
			const ids = new Set(Object.keys(byId));
			if (statusMap) for (const id of statusMap.keys()) ids.add(id);
			for (const sid of ids) {
				const entry = byId[sid] || {};
				const st = (statusMap && statusMap.get(sid)) || {};
				const running = st.running !== undefined ? st.running : entry.running;
				let pendingInteraction = st.pendingInteraction;
				if (pendingInteraction === undefined && legacyPending) pendingInteraction = legacyPending.get(sid);
				if (pendingInteraction === undefined) pendingInteraction = entry.pendingInteraction;
				const completionUnread = st.completionUnread === true;
				const completed = !!entry.completed;
				if (entry.blank) {
					prevRunning.set(sid, running);
					lastCompleted.set(sid, completed);
					lastUnread.set(sid, completionUnread);
					continue;
				}
				const prevRun = prevRunning.get(sid);
				const prevDone = lastCompleted.get(sid);
				const prevUnread = lastUnread.get(sid);
				prevRunning.set(sid, running);
				lastCompleted.set(sid, completed);
				lastUnread.set(sid, completionUnread);

				if (pendingInteraction) {
					const key = sid + ":confirm:" + confirmKey(pendingInteraction);
					if (lastNotifiedKeys.get(sid) === key) continue;
					lastNotifiedKeys.set(sid, key);
					const title = entry.title || cwdBasename(entry.cwd) || "DSH task";
					reportNotify(title, confirmText(pendingInteraction), "confirm", sid);
					continue;
				}

				const runningEdge = prevRun === true && running === false;
				const completedEdge = completed && prevDone === false;
				const catchUp = completed && prevDone === undefined;
				// 从未观察到该会话 running 状态时(如页面刚加载就已完成)没有
				// running 边沿可用,用 dsh 的「后台完成未读」标记兜底;已观察到的
				// 会话一律走 runningEdge,否则同一次停止会被两条判定各报一次。
				const unreadEdge = prevRun === undefined && completionUnread && prevUnread !== true;
				if (runningEdge || completedEdge || catchUp || unreadEdge) {
					const title = entry.title || cwdBasename(entry.cwd) || "DSH task";
					const bodyText = entry.title ? "" : (entry.cwd || "");
					reportNotify(title, bodyText, "complete", sid);
				}
			}
			// 仍在 uiSession 状态里的会话不能清:清了下一轮 prev* 又是 undefined,
			// 同一次待确认/未读会被反复重报(statusMap-only 的会话就是这种情形)。
			const stillTracked = (sid) => !!byId[sid] || !!(statusMap && statusMap.has(sid));
			for (const sid of [...prevRunning.keys()]) {
				if (!stillTracked(sid)) {
					prevRunning.delete(sid);
					lastCompleted.delete(sid);
					lastUnread.delete(sid);
					lastNotifiedKeys.delete(sid);
				}
			}
			for (const sid of [...lastNotifiedKeys.keys()]) {
				if (!stillTracked(sid)) lastNotifiedKeys.delete(sid);
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

		// 「附加为上下文」末端:把文件路径以 dsh `@` 引用芯片插入当前会话输入框。
		// 契约:函数存在且返回 true ⇒ 芯片已插入/重试中;返回 false ⇒ 调用方回退纯文本注入。
		// 依据 specs/dsh-file-context-insert/PLAN-file-reference-chip.md(dsh 0.1.2-rc.1)。
		function installFileReferenceInjection(rootCtx, sessions) {
			if (window.__zapInsertFileReference) return;
			// dsh-file-reference formatFileMention 的等价移植:
			// 含引号/控制字符 → 无法表示;含空白 → @"path";否则 @path。
			function mentionFor(clean, isDir) {
				const path = isDir ? clean + "/" : clean;
				if (/[\u0000-\u001f\u007f-\u009f"]/.test(path)) return null;
				if (!/\s/.test(path)) return "@" + path;
				return isDir ? '@"' + path : '@"' + path + '"';
			}
			// @ 引用的规范形态是 workspace-root 相对(dsh FILE_REFERENCE_PROMPT);
			// 文件不在会话 cwd 下时保留绝对路径(模型仍可 read)。
			function relativeUnder(dir, fullPath) {
				if (!dir) return undefined;
				const norm = (s) => String(s).replace(/\\/g, "/").replace(/\/+$/, "");
				const d = norm(dir);
				const f = norm(fullPath);
				return f === d ? undefined : (f.startsWith(d + "/") ? f.slice(d.length + 1) : undefined);
			}
			window.__zapInsertFileReference = function (fullPath, isDir) {
				try {
					const snap = sessions.list.getSnapshot();
					if (snap.phase !== "ready") return false;
					// 当前会话与 reportCurrentPath 同源:0.1.6-alpha.2 起快照不再带
					// `current`,统一走 currentSessionId(见其注释)。
					const sessionId = currentSessionId(snap);
					if (!sessionId) return false;
					const conversation = rootCtx.get("conversation");
					const input = conversation && conversation.input;
					if (!input) return false;
					const shell = input.shell(sessionId); // binding 缺失时 throw → 落入 catch
					const cwd = snap.byId[sessionId] && snap.byId[sessionId].cwd;
					const clean = relativeUnder(cwd, fullPath) || String(fullPath).replace(/[\\/]+$/, "");
					const mention = mentionFor(clean, isDir === true);
					if (!mention) return false;
					const label = (clean.split(/[\\/]/).pop() || clean) + (isDir === true ? "/" : "");
					const reference = {
						source: "reference",
						ref: mention,
						label: label,
						appearance: isDir === true ? "folder" : "file",
						clipboardText: mention
					};
					// insertReference 要求 phase ∈ {plain, claimed} 且 span.draftRev === shell.rev;
					// span 必须是 detect 坐标(caretSpan 产物),不能拿 compose().draft(clipboard 坐标)当 span。
					// 返回 false ⇒ 提交窗口期(phase busy),逐帧重试;耗尽仅告警,本次放弃。
					let tries = 0;
					const attempt = () => {
						try {
							const st = shell.compose();
							const caret = shell.caretSpan();
							const span = { start: caret.start, end: caret.end, draftRev: st.draftRev };
							if (shell.insertReference(reference, span)) return true;
						} catch (err) {
							console.error("[zap-bridge-client] insertFileReference failed:", err);
							return false;
						}
						if (++tries > 8) {
							console.warn("[zap-bridge-client] insertFileReference: composer busy, giving up");
							return false;
						}
						requestAnimationFrame(attempt);
						return true; // 重试中:对外仍视为已受理,不触发 Rust 回退
					};
					return attempt();
				} catch (err) {
					console.error("[zap-bridge-client] insertFileReference failed:", err);
					return false;
				}
			};
		}

		// ── 文件链接拦截:统一改在 Zap 内打开 ──
		// dsh 0.1.5 起,聊天 UI 的所有文件入口(工具卡片 fileLink、ProducedFiles/
		// deliverables chips、markdown file mention)经 chat 的 openFile face 收敛到
		// `ctx.sidebarRight.openResource("dsh-resource://file/…")`——旧版收敛点
		// `ctx.remote.session.openWorkspacePath` 已无 UI 调用方(dsh-api-remotes
		// 仅保留协议端点)。Zap 内嵌场景 patch 此公共 face 改发 `zap.open_file`
		// IPC,由 Zap 按 Notebook/Editor/Session 分类在 Zap 内打开。侧栏文件树的
		// onOpen 走 tabActions → openResourceIn(内部路径,不经此 face),其 dsh
		// 原生预览不受影响。
		/// 文件地址前缀(dsh-util/workspace-path 的 file-address.ts)。
		const FILE_ADDRESS_PREFIX = "dsh-resource://file/";

		/// 把地址里的 path 归一成本机绝对路径:已是绝对路径直接返回,工作区相对
		/// 路径按该会话的根(cwd)拼接,cwd 取不到时回退到上报过的项目路径。
		function absoluteFilePath(path, sessionId) {
			if (path.startsWith("/") || /^[A-Za-z]:[/\\]/.test(path) || path.startsWith("\\\\")) {
				return path;
			}
			const cwd = sessionsRef?.list.getSnapshot().byId[sessionId]?.cwd;
			const root = cwd || lastReportedPath;
			if (!root) return undefined;
			return root.replace(/\/+$/, "") + "/" + path;
		}

		/// 解析 `dsh-resource://file/…` 地址为文件绝对路径;非文件地址、或无法
		/// 定位到本机路径时返回 undefined,交回原方法。
		/// 段语义逐行对齐 dsh-util-workspace-path 的 parseFileAddress:session 形态
		/// 取 id + 其余段(首段为空即绝对路径);absolute 形态单斜杠补前导 `/`、
		/// 双斜杠还原 UNC、盘符首段保持字面;`?`/`#` 之后一律截断。
		function resolveFileAddressPath(address) {
			try {
				if (typeof address !== "string" || !address.startsWith(FILE_ADDRESS_PREFIX)) return undefined;
				const end = address.search(/[?#]/);
				const [scope, ...rest] = address
					.slice(FILE_ADDRESS_PREFIX.length, end === -1 ? undefined : end)
					.split("/");
				if (scope === "session") {
					const [id, ...segments] = rest;
					if (id === undefined || id === "" || segments.length === 0) return undefined;
					const path = segments.map(decodeURIComponent).join("/");
					// 空路径即工作区根(目录):交给 dsh 原生处理。
					if (path === "") return undefined;
					return absoluteFilePath(path, decodeURIComponent(id));
				}
				if (scope === "absolute") {
					const unc = rest[0] === "" && rest.length > 1;
					const segments = (unc ? rest.slice(1) : rest).map(decodeURIComponent);
					if (segments.length === 0 || segments[0] === "") return undefined;
					if (unc) return `//${segments.join("/")}`;
					return /^[A-Za-z]:$/.test(segments[0]) ? segments.join("/") : `/${segments.join("/")}`;
				}
				return undefined;
			} catch {
				return undefined;
			}
		}

		function installOpenFileInterceptor(ctx) {
			const sidebarRight = ctx.sidebarRight;
			if (!sidebarRight || typeof sidebarRight.openResource !== "function") {
				console.error("[zap-bridge-client] ctx.sidebarRight.openResource unavailable; file links stay in dsh");
				return;
			}
			const origOpenResource = sidebarRight.openResource;
			sidebarRight.openResource = function (address, options) {
				const path = resolveFileAddressPath(address);
				if (path === undefined) return origOpenResource.call(this, address, options);
				// 与 zap-bridge 现有通知类 IPC 一致:不等 Zap ack(点击即时生效,
				// 失败仅记日志)。
				zapRpc('zap.open_file', { path }).catch((err) => {
					console.error("[zap-bridge-client] open_file failed:", err);
				});
				console.log("[zap-bridge-client] open_file ->", path);
			};
			ctx.effect(() => () => {
				sidebarRight.openResource = origOpenResource;
			}, "zap-bridge-client: open-file interceptor");
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
			bridgeCtx = ctx;
			// 「附加为上下文」末端:暴露结构化 @ 引用芯片插入,供 Rust 侧 evaluate_script 调用。
			installFileReferenceInjection(ctx, sessions);
			// 会话切换末端:Rust 侧通知点击经 evaluate_script 调用。
			// dsh 0.1.6-alpha.2 起 sessions 服务不再提供 open()(其公开方法表里没有,
			// dsh 全库也零调用),"切主会话"改由公开的 uiWorkspace.openSession(与侧栏
			// 点击同源)。此处惰性解析 uiWorkspace:本函数在 apply 之后才被点击触发,
			// 不存在装载顺序依赖;≤0.1.6-alpha.1 回退 sessions.open。
			window.__zapActivateSession = function (sessionId) {
				if (!sessionId) return;
				const uiWorkspace = ctx.get("uiWorkspace");
				if (typeof uiWorkspace?.openSession === "function") {
					uiWorkspace.openSession(sessionId);
					return;
				}
				if (sessionsRef && typeof sessionsRef.open === "function") {
					sessionsRef.open(sessionId);
					return;
				}
				// 两个入口都不可用时留痕,避免下次再以"点击无反应"的形态静默复发。
				console.warn("[zap-bridge-client] activate_session: no session-switch entry available");
			};
			// 文件链接拦截:sidebarRight.openResource → Zap 内打开。
			installOpenFileInterceptor(ctx);

			const unsubSessions = sessions.list.subscribe(reportCurrentPath);
			const unsubWorkspaces = workspaces.list.subscribe(reportCurrentPath);
			const unsubFileExplorer = workspaces.list.subscribe(scheduleSyncFileExplorerButtons);
			const unsubNotify = sessions.list.subscribe(checkNotify);
			// 「需确认」不脏 sessions list,另订阅 uiSession 的状态源;服务此刻若
			// 未就绪,由 checkNotify 内的 ensureStatusSubscription 补建。
			ensureStatusSubscription();
			reportCurrentPath();
			checkNotify();
			scheduleSyncFileExplorerButtons();
			const observer = new MutationObserver(() => scheduleSyncFileExplorerButtons());
			observer.observe(document.body, { childList: true, subtree: true });

			ctx.effect(() => {
				return () => {
					unsubSessions();
					unsubWorkspaces();
					if (statusUnsub) {
						statusUnsub();
						statusUnsub = undefined;
					}
					unsubFileExplorer();
					unsubNotify();
					if (pendingSyncRaf) cancelAnimationFrame(pendingSyncRaf);
					observer.disconnect();
					document.querySelectorAll('[data-zap-file-explorer]').forEach(el => el.remove());
					delete window.__onZapResponse;
					delete window.__zapInsertFileReference;
					delete window.__zapActivateSession;
				};
			});
		}

		exports.apply = apply;
		exports.inject = inject;
		return module.exports;
	}
});
