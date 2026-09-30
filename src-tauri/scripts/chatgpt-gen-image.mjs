#!/usr/bin/env node
// 用 CDP 驱动"隔离 profile 里已登录的 ChatGPT"生图，并把原图取回本地。
//
// 为什么需要它：ChatGPT 桌面端/网页没有生图 API，而宠物素材（走路雪碧图）必须靠它生成。
// 走 CDP 的好处是零焦点抢占（不点鼠标、不切 Space），且每一步都可用 Runtime.evaluate 复核。
//
// 前置（一次性）：
//   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//     --user-data-dir="$HOME/.costdog/chrome-profile" \
//     --remote-debugging-port=9222 --no-first-run --no-default-browser-check \
//     "https://chatgpt.com/"
//   注意：Chrome 136+ 不允许对默认 profile 开调试端口，必须用独立 user-data-dir。
//   注意：9333 端口被 ego lite 占用，别用。
//
// 用法：
//   node chatgpt-gen-image.mjs --ref pet-cat-run.png --out /tmp/cat-walk-raw.png \
//        --prompt "生成一张图片：橘猫侧面走路循环精灵图（sprite sheet），横向排列8帧，从左到右构成一个完整的步行周期（迈前腿→四腿交替→蹬地→回收）。严格保持参考图里这只橘猫的角色形象、配色、渲染风格与大小比例完全一致，纯白背景，帧间等距，每帧姿态基线一致，无文字、无边框、无地面阴影。"
//   （--ref 可省略；带参考图能显著提高"同一只角色"的一致性。生成约 40~70s，成品 2172×724。）
//   拿到原图后用 scripts/make-walk-sheet.py 切成 pet.html 用的雪碧图。
//
// 实测经验（2026-09-30）：
//   * 成品尺寸稳定为 2172×724（8 帧横排）；模型自带的等分布局很准（段宽差 <3%），
//     所以切帧优先按等分列切，保留原图对齐，不要逐帧 bbox 居中（腿摆动会让身体左右抖）。
//   * 提示词里写"渲染风格与参考图一致"仍可能出像素风（边缘对比度 15~21 vs 参考图 4），
//     想要非像素风需要更强的措辞或接受与睡姿帧的风格差。
//   * 页面里的 composer 不是 #prompt-textarea（已改名），用 div[contenteditable="true"]。
//   * 生成结果在页面上是 blob: URL，且会出现两份；用 canvas 画出来后 toDataURL 取回（同源，不脏画布）。
//   * 参考图缩略图也是 img，宽高比成品小 —— 等待时要按 naturalWidth 过滤，否则会抓到参考图。

const argv = process.argv.slice(2);
const opt = (name, def) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : def; };
const PORT = Number(opt('--port', 9222));
const refPath = opt('--ref', null);
const prompt = opt('--prompt', null);
const out = opt('--out', null);
const MINW = Number(opt('--minw', 1500));
const timeoutSec = Number(opt('--timeout', 300));
if (!prompt || !out) { console.error('用法: --prompt "<提示词>" --out <输出.png> [--ref <参考图>] [--port 9222] [--minw 1500] [--timeout 300]'); process.exit(1); }

const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = targets.find(t => t.type === 'page' && /chatgpt\.com/.test(t.url));
if (!page) { console.error('找不到 chatgpt.com 标签页——按文件头的命令启动 Chrome'); process.exit(1); }
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0; const pending = new Map();
ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
await new Promise(r => ws.addEventListener('open', r));
const send = (method, params = {}) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async expr => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error(String(r.result.exceptionDetails.exception?.description || '').slice(0, 300));
  return r.result?.result?.value;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));

// 1) 新会话
await ev("location.href='https://chatgpt.com/'; 'go'");
await sleep(7000);

// 2) 上传参考图（隐藏 input[type=file]，setFileInputFiles 不需要可见）
if (refPath) {
  const doc = await send('DOM.getDocument', { depth: -1 });
  const q = await send('DOM.querySelector', { nodeId: doc.result.root.nodeId, selector: 'input[type=file]' });
  if (!q.result?.nodeId) { console.error('页面里没有 input[type=file]'); process.exit(2); }
  await send('DOM.setFileInputFiles', { nodeId: q.result.nodeId, files: [refPath] });
  await sleep(2500);
  console.log('已上传参考图:', refPath);
}

// 3) 聚焦 composer（注意：不是 #prompt-textarea）→ 输入 → 回车发送
await ev("document.querySelector('div[contenteditable=\"true\"]').focus(); 'focused'");
await send('Input.insertText', { text: prompt });
await sleep(1500);
const composed = await ev("document.querySelector('div[contenteditable=\"true\"]').innerText.length");
console.log('composer 文本长度:', composed);
if (!composed || composed < 10) { console.error('提示词没进 composer，放弃'); process.exit(2); }

// 发送并**校验真的发出去了**：只看回车不可靠 —— 参考图上传失败时页面顶部会挂一条
// "上传未成功"，此时回车被吞掉（实测：composer 里留着整段提示词，页面零条消息）。
// 所以先点发送按钮，1.2s 后看 composer 是否清空；没清空再退回回车；仍不行就
// 开新会话（顺带清掉坏附件）重试一次。
async function composerLength() {
  return await ev("(document.querySelector('div[contenteditable=\"true\"]')||{innerText:''}).innerText.length");
}
async function sendViaButton() {
  return await ev(`(()=>{const b=document.querySelector('button[data-testid="send-button"],button[aria-label*="Send"],button[aria-label*="发送"]');if(b&&!b.disabled){b.click();return 'clicked';}return 'no-button';})()`);
}
async function pressEnter() {
  // 必须带 text/unmodifiedText:'\r'：这个版本的 ChatGPT 没有 data-testid="send-button"
  // 的发送按钮（composer 附近只有 附件/模型/听写/语音 四个），而**不带 text 的合成 Enter
  // 会被内容可编辑区当成普通按键吞掉**——实测 composer 里留住整段提示词、页面零条消息。
  // 带上 '\r' 后回车才真的提交（实测 composer 清空、消息出现在会话里）。
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
}
// 发送成功的判据要**宽松且多路**：composer 清空可能滞后好几秒，只等 1.2s 会误判成
// "没发出去"，然后开新会话把刚发起的生成中断（实测踩过：提示词其实已送达、页面在
// "思考中"，脚本却以为失败并跳走）。所以：清空 OR 出现停止按钮 OR 页面里出现了
// 刚输入的提示词片段，任一成立即算发出；最多轮询 10s。
async function sentEvidence() {
  const composer = await composerLength();
  if (composer === 0) return 'composer 清空';
  const stop = await ev(`!!document.querySelector('button[data-testid="stop-button"],button[aria-label*="停止"],button[aria-label*="Stop"]')`);
  if (stop) return '出现停止按钮';
  // 注意：不能用 body.innerText 找提示词 —— **输入框里的文字本身就在 innerText 里**，
  // 于是"没发出去"也会被判成已发送（实测踩过：提示词卡在 composer，脚本却报发送成功）。
  // 只认真正进了会话的消息节点。
  const echoed = await ev(`(()=>{
    const t=${JSON.stringify(prompt.slice(0, 24))};
    return [...document.querySelectorAll('[data-message-author-role], article, .markdown')]
      .some(e=>e.innerText && e.innerText.includes(t));
  })()`);
  if (echoed) return '消息已出现在会话里';
  return null;
}
async function attemptSend(step) {
  const how = await sendViaButton();
  if (how === 'no-button') await pressEnter();
  for (let i = 0; i < 20; i++) {
    await sleep(500);
    const why = await sentEvidence();
    if (why) { console.log(`已发送（${step}: ${how === 'no-button' ? 'Enter' : '发送按钮'} / ${why}）`); return true; }
  }
  console.log(`${step} 没发出去（composer 仍有文本且无生成迹象）`);
  return false;
}
const sent = await attemptSend('第1次');
if (!sent) { console.error('提示词没能发出，放弃（不再自动换会话，避免中断可能已开始的生成）'); process.exit(2); }

// 4) 等新图（基线在发送后取，按 naturalWidth 过滤掉参考图缩略图）
await sleep(8000);
const baseline = new Set(JSON.parse(await ev(`JSON.stringify([...document.querySelectorAll('img')].filter(i=>i.naturalWidth>=${MINW}).map(i=>i.src))`)));
const t0 = Date.now();
let found = null;
while ((Date.now() - t0) / 1000 < timeoutSec) {
  const st = JSON.parse(await ev(`JSON.stringify({busy: !!document.querySelector('button[data-testid="stop-button"], [aria-label*="停止"]'), imgs: [...document.querySelectorAll('img')].filter(i=>i.naturalWidth>=${MINW}).map(i=>({w:i.naturalWidth,h:i.naturalHeight,src:i.src}))})`));
  const cand = st.imgs.filter(i => !baseline.has(i.src));
  console.log(`[${((Date.now() - t0) / 1000).toFixed(0)}s] busy=${st.busy} 大图=${st.imgs.length} 新=${cand.length} ${cand.map(c => c.w + 'x' + c.h).join(',')}`);
  if (cand.length && !st.busy) { found = cand[cand.length - 1]; break; }
  if (cand.length && (Date.now() - t0) / 1000 > 90) { found = cand[cand.length - 1]; break; }
  await sleep(3000);
}
if (!found) { console.error('超时：没等到生成图'); process.exit(1); }

// 5) canvas → dataURL → 落盘（blob: 同源，画布不会被污染）
const dataUrl = await ev(`(()=>{const im=[...document.querySelectorAll('img')].find(i=>i.src===${JSON.stringify(found.src)}); if(!im) return 'NO_IMG'; const c=document.createElement('canvas'); c.width=im.naturalWidth; c.height=im.naturalHeight; c.getContext('2d').drawImage(im,0,0); try{return c.toDataURL('image/png')}catch(e){return 'TAINTED:'+e.message}})()`);
if (!String(dataUrl).startsWith('data:image')) { console.error('提取失败:', String(dataUrl).slice(0, 200)); process.exit(2); }
const fs = await import('node:fs');
fs.writeFileSync(out, Buffer.from(dataUrl.split(',')[1], 'base64'));
console.log(`已保存 ${out} (${found.w}x${found.h}, ${fs.statSync(out).size} bytes)`);
console.log('下一步: python3 scripts/make-walk-sheet.py ' + out + ' src-tauri/embedded/<pet>-walk-sheet.png');
ws.close();
