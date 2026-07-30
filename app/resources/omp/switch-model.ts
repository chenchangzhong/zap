// zap-omp-switch-model v1 — 此标记用于检测扩展是否已安装
import { unlinkSync, existsSync } from "node:fs";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const isWin = process.platform === "win32";
// socket 路径按 session 命名，避免多 omp 进程抢占同一 socket 导致串号。
// session_id 在 session_start 时确定；缺失时回退到全局路径（兼容旧客户端）。
function sockPathFor(sessionId) {
  const name = sessionId ? `omp-model-switch-${sessionId}.sock` : "omp-model-switch.sock";
  return isWin ? join(tmpdir(), name)
    : join(homedir(), ".omp", "agent", sessionId ? `model-switch-${sessionId}.sock` : "model-switch.sock");
}
let sockPath = sockPathFor(null);

let pi = null;
let server = null;
let allModels = [];
let sessionCtx = null;
let switchModelHandler = null;
let lastReportedModel = null;

// 把 model 对象归一化为 "provider/id" 形式，与 Zap registry 的 selector 对齐。
function modelKey(m) {
  if (!m) return null;
  const p = String(m.provider ?? "").toLowerCase();
  const id = String(m.id ?? "").toLowerCase();
  if (!p && !id) return null;
  return `${p}/${id}`;
}

// 读取当前模型对象（不同 omp 版本方法名可能不同，做防御式获取）。
function currentModel(ctx) {
  const models = ctx?.models;
  if (!models) return null;
  if (typeof models.current === "function") return models.current();
  if (typeof models.getCurrent === "function") return models.getCurrent();
  return null;
}

// 通过 OSC 777 向 Zap 上报模型变更（title=warp://cli-agent，body 为 JSON）。
function reportModelChange(model) {
  const key = modelKey(model);
  if (!key) return;
  const body = JSON.stringify({ v: 1, agent: "omp", event: "model_change", model: key });
  process.stdout.write(`\x1b]777;notify;warp://cli-agent;${body}\x07`);
  lastReportedModel = key;
}

async function switchTo(selector, ctx) {
  const lower = selector.toLowerCase();
  const candidates = ctx ? ctx.models.list() : allModels;

  // Exact match: "provider/id" -> provider + id
  const parts = lower.split("/");
  const byProviderId = parts.length === 2
    ? candidates.find(m => {
        const mid = String(m.id ?? "").toLowerCase();
        const mp = String(m.provider ?? "").toLowerCase();
        return mp === parts[0] && (mid === parts[1] || mid.endsWith("/" + parts[1]));
      })
    : undefined;

  // Substring fallback
  const model = byProviderId ?? candidates.find(m => {
    const fields = [m.id, m.name, m.selector].filter(Boolean);
    return fields.some(f => String(f).toLowerCase().includes(lower));
  }) ?? candidates.find(m => {
    const id = String(m.id ?? "").toLowerCase();
    const p = String(m.provider ?? "").toLowerCase();
    return id.includes(lower) || p.includes(lower) || `${p}/${id}`.includes(lower) || `${p} ${id}`.includes(lower);
  });

  if (!model) { ctx?.ui.notify(`Unknown: "${selector}"`, "error"); return; }
  await pi.runtime.setModel(model);
  reportModelChange(model);
  ctx?.ui.notify(`\u2192 ${model.name ?? model.id}`, "info");
}

async function pickAndSwitch(ctx) {
  const models = allModels.length ? allModels : ctx.modelRegistry.getAvailable();
  if (!models.length) { ctx.ui.notify("No models with auth", "warning"); return; }
  const picked = await ctx.ui.select("Select model", models.map(m => `${m.name ?? m.id} (${m.id})`));
  if (!picked) return;
  const idx = models.findIndex(m => `${m.name ?? m.id} (${m.id})` === picked);
  if (idx === -1) return;
  await pi.runtime.setModel(models[idx]);
  reportModelChange(models[idx]);
  ctx.ui.notify(`\u2192 ${models[idx].name ?? models[idx].id}`, "info");
}

function startSocket() {
  stopSocket();
  if (existsSync(sockPath)) {
    try { unlinkSync(sockPath); } catch { /* ignore */ }
  }
  server = createServer((sock) => {
    let buf = "";
    sock.on("data", async (chunk) => {
      buf += chunk.toString("utf8");
      let idx;
      while ((idx = buf.indexOf("\n")) !== -1) {
        const line = buf.slice(0, idx).trim();
        buf = buf.slice(idx + 1);
        if (!line) continue;
        try {
          const msg = JSON.parse(line);
          if (msg.model && switchModelHandler && sessionCtx) {
            await switchModelHandler(msg.model, sessionCtx);
          }
        } catch { /* ignore */ }
      }
    });
    sock.on("error", () => {});
  });
  server.on("error", (err) => {
    if (err.code === "EADDRINUSE") {
      unlinkSync(sockPath);
      startSocket();
    }
  });
  server.listen(sockPath);
}

function stopSocket() {
  if (server) {
    try { server.close(); } catch { /* ignore */ }
    server = null;
  }
}

export default function (ext) {
  pi = ext;

  pi.on("session_start", async (_event, ctx) => {
    sessionCtx = ctx;
    allModels = ctx.modelRegistry.getAvailable();
    // 用该 session 的 id 命名 socket，与 Zap 侧 session_context.session_id 对齐。
    let sessionId = null;
    try { sessionId = ctx?.sessionManager?.getSessionId?.() ?? null; } catch { /* ignore */ }
    sockPath = sockPathFor(sessionId);
    startSocket();
    // 初始化基线，避免启动时误报。
    lastReportedModel = modelKey(currentModel(ctx));
  });

  // omp 内置 /switch 等切换不 emit 扩展事件，只能在 turn_start/agent_start 轮询
  // ctx.models.current() 对比上次值来感知变化（下次交互时补上报）。
  const pollModel = async (_event, ctx) => {
    const m = currentModel(ctx);
    const key = modelKey(m);
    if (key && key !== lastReportedModel) {
      reportModelChange(m);
    }
  };
  pi.on("turn_start", pollModel);
  pi.on("agent_start", pollModel);

  pi.on("session_shutdown", () => {
    allModels = [];
    sessionCtx = null;
    stopSocket();
  });

  const cmd = {
    description: "Switch model. /switch-model <name> or /switch-model to pick",
    getArgumentCompletions: (prefix) =>
      allModels
        .filter(m => `${m.name ?? m.id}`.toLowerCase().includes(prefix.toLowerCase()))
        .map(m => ({ value: m.id, label: `${m.name ?? m.id} (${m.id})` })),
    handler: async (args, ctx) => {
      if (args) {
        await switchTo(args, ctx);
      } else {
        await pickAndSwitch(ctx);
      }
    },
  };
  switchModelHandler = cmd.handler;
  pi.registerCommand("switch-model", cmd);
}
