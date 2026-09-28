// CostDog 宠物注入器：向 Electron 客户端（ZCode/Codex）注入常驻宠物 UI。
// 用法：node costdog-inject.mjs --port <cdp端口> --host <zcode|codex> [--data 9401]
const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]"]);

function parseArgs(argv) {
  const o = { port: 9345, data: 9401, host: "zcode" };
  for (let i = 2; i < argv.length; i++) {
    if (argv[i] === "--port") o.port = Number(argv[++i]);
    else if (argv[i] === "--data") o.data = Number(argv[++i]);
    else if (argv[i] === "--host") o.host = argv[++i];
  }
  return o;
}

const opts = parseArgs(process.argv);

const THEMES = {
  zcode: { plateBg: "rgba(28,28,34,.94)", plateBorder: "rgba(255,255,255,.16)", text: "#e8e8e8", accent: "#f97316", bottom: 52 },
  codex: { plateBg: "rgba(28,28,34,.94)", plateBorder: "rgba(255,255,255,.16)", text: "#d4d4d4", accent: "#f97316", bottom: 44 },
};

async function getPageTarget(port) {
  const res = await fetch(`http://127.0.0.1:${port}/json`);
  if (!res.ok) throw new Error(`CDP HTTP ${res.status}`);
  const targets = await res.json();
  const pages = targets.filter(t => t.type === "page" && t.webSocketDebuggerUrl);
  if (!pages.length) throw new Error("no page target");
  // 优先主界面（非 devtools/扩展页）
  return pages.find(t => (t.url || "").startsWith("http")) || pages[0];
}

class CdpSession {
  constructor(target, port) {
    const url = new URL(target.webSocketDebuggerUrl);
    if (url.protocol !== "ws:" || !LOOPBACK.has(url.hostname)) throw new Error("non-loopback CDP URL rejected");
    this.ws = new WebSocket(url.href);
    this.nextId = 1;
    this.pending = new Map();
    this.ws.addEventListener("message", ev => {
      const msg = JSON.parse(ev.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      }
    });
    this.ready = new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve);
      this.ws.addEventListener("error", () => reject(new Error("ws error")));
    });
  }
  send(method, params) {
    const id = this.nextId++;
    this.ws.send(JSON.stringify({ id, method, params: params || {} }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
  async evaluate(expression) {
    const r = await this.send("Runtime.evaluate", { expression, awaitPromise: false, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text);
    return r.result.value;
  }
  close() { this.ws.close(); }
}

const theme = THEMES[opts.host] || THEMES.zcode;
const DATA = `http://127.0.0.1:${opts.data}`;

const payload = `(function(){
  if (window.__COSTDOG_PET) return "already";
  window.__COSTDOG_PET = true;
  var css = document.createElement("style");
  css.textContent = [
    "#costdog-pet{position:fixed;right:26px;bottom:${theme.bottom}px;z-index:2147483000;pointer-events:none;display:flex;flex-direction:column;align-items:center;gap:2px;font-family:ui-monospace,Menlo,monospace}",
    "#costdog-pet .plate{background:${theme.plateBg};border:1px solid ${theme.plateBorder};border-radius:10px;padding:2px 8px;font-size:10px;color:${theme.text};white-space:nowrap;box-shadow:0 3px 10px rgba(0,0,0,.35)}",
    "#costdog-pet .plate b{color:${theme.accent};font-weight:700}",
    "#costdog-pet .plate .sep{color:rgba(255,255,255,.22);margin:0 3px}",
    "#costdog-pet img{height:54px;width:auto;display:block}",
    "#costdog-pet.sleep img{content:url('${DATA}/pet-sleep.png')}"
  ].join("\\n");
  document.head.appendChild(css);
  var root = document.createElement("div");
  root.id = "costdog-pet";
  root.innerHTML = '<div class="plate"><b id="cdp-rate">—</b><span class="sep">│</span><span id="cdp-cost">$0</span><span class="sep">│</span><span id="cdp-ctx">CTX —</span></div><img src="${DATA}/pet-run.png" alt="">';
  (document.body || document.documentElement).appendChild(root);
  setInterval(function(){
    fetch("${DATA}/stats.json").then(function(r){return r.json();}).then(function(s){
      var row = s.sources.find(function(x){return x.source==="${opts.host==="chatgpt"?"codex":opts.host}";}) || s.sources[0];
      if (!row) return;
      var live = row.tokens_per_min > 0.05;
      document.getElementById("cdp-rate").innerHTML = live ? ("▲ " + Math.round(row.tokens_per_min/60) + "/s") : "zzz";
      document.getElementById("cdp-cost").textContent = "$" + (s.today_cost >= 100 ? s.today_cost.toFixed(0) : s.today_cost.toFixed(2));
      document.getElementById("cdp-ctx").textContent = "CTX " + (row.last_context_tokens >= 1e6 ? (row.last_context_tokens/1e6).toFixed(1)+"M" : Math.round(row.last_context_tokens/1000)+"k");
      root.className = live ? "" : "sleep";
    }).catch(function(){});
  }, 2000);
  return "injected";
})()`;

const target = await getPageTarget(opts.port);
const session = new CdpSession(target, opts.port);
await session.ready;
const result = await session.evaluate(payload);
console.log(`[CostDog] injected into ${opts.host}: ${result}`);
session.close();
process.exit(0);
