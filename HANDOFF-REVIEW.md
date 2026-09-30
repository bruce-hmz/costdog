# CostDog 交接文档 Review 报告

> 对象：`HANDOFF.md`（2026-09-29，ZCode agent）与当前 `main`（2c48ebe，ahead origin/main **64**）。
> 方法：读码 + 本地真值查询（`~/.costdog/costdog.sqlite`、`~/.zcode/cli/db/db.sqlite`）+ 运行中实例的 `/pet.json`、`/stats.json`、`lsof`、`netstat`、`unified log` + 编译 ObjC 探针核对 AppKit 常量/所有权 + 像素分析素材 + `cargo test`。
> 复核者：DSH review agent。**未能**做屏幕截图（本进程无屏幕录制授权，`screencapture` 报 `could not create image from display`），因此涉及"屏幕上看起来怎样"的结论均由 DOM/CSS/素材像素/系统日志推得，已在各条标注证据强度。

---

## 0. 结论摘要

文档把 9 条"血泪铁律"和 65 节迭代记录交代得很完整，工程纪律（注释写 why、测试钉行为）也在。但**三条被文档宣布为"已修复"的核心问题实际不成立，另有一条 P0 级功能是死的**，且它们全部落在测试盲区（没有一条测试读 `pet.html`）：

| # | 问题 | 文档声称 | 实际 |
|---|---|---|---|
| P0-1 | 睡姿精灵永不显示 → 静默 60s 后狗彻底消失 | §3"静默 60s 趴睡"、§65"睡觉隐身三修" | `pet.html:41` 的 `hidden` 属性无人移除，sleep 态两个精灵都被隐藏 |
| P0-2 | 走路雪碧图被声明成 8 倍大 → 走路时是一条涂抹 | §58/§66"8 帧走路循环/steps 帧动画" | `background-size:5552px` 作用在 694px 宽素材上，8× 拉伸 + 3× 压扁 |
| P0-3 | App Nap 实际没被禁用 | §69"NSProcessInfo beginActivity 禁用…验证 nap disabled ✓" | 返回的 activity token 是 +0 autoreleased，指针一丢活动即结束；位掩码也没设上 LatencyCritical |
| P0-4 | 托盘"召唤小狗"自救开关 | §68"实测链路全通" | `pet.html` 的 `fetch(127.0.0.1:9401/pet.json)` 被自家 CSP `connect-src` 拦掉，前端收不到 summon 计数 |
| P1-1 | zcode 速率"生成中实时" | §5/§36 | 查询过滤掉 `status='running'`，且 completed 行 `start+duration == completed_at`（28791/28791 行成立）→ 实时分支是死代码，只有"完成后 45s" |
| P1-2 | 缓存命中率分母按源修好 | §35"命中率 4752% 修复" | 只修了 codex；claude-code 实测 1432%、opencode 1204%，面板会显示 >1000% |

---

## P0（影响产品核心承诺）

### P0-1 静默 60s 后狗 100% 不可见 —— 睡姿精灵被 `hidden` 永久隐藏

`embedded/pet.html`：

- `:41` `<img id="dog-sleep" class="sprite" src="pet-sleep.png" alt="" hidden>`
- `:30` `img.sprite[hidden]{display:none}`
- `:31` `body[data-state="sleep"] #dog-run{display:none}`
- 全文再无任何 `removeAttribute` / 对 `#dog-sleep` 的引用（grep 只有第 8/30/31/32/41 行提到 sleep）。
- `:62` `sleeping = !liveNow() && now-lastLive>60000` → 静默 60s 进入 `data-state="sleep"`。

后果：**sleep 态下两个精灵都是 `display:none`**，屏幕上只剩铭牌与 Zzz。对"成本监控"这种绝大多数时间静默的工具，这就是常态而非边角。

运行期旁证：我连续 12.5s 采样 `/pet.json`，`x` 恒为 1421（状态机被 `if(!sleeping)` 整段跳过）→ 该实例此刻正处于 sleep 分支。按代码推断，**现在屏幕上没有狗**。

影响：这最可能就是用户反复报的"狗不见了/间歇隐身"的主因（活动期采样自然看得到 = "同刻实拍可见"），而此前 ~50 个提交都在查 window server 合成。

修（1 行）：删 `:41` 的 `hidden` 与 `:30` 的规则，可见性只由 `:31/:32` 控制；补一条 `body:not([data-state="sleep"]) #dog-sleep{display:none}`。

### P0-2 走路精灵是一条 88×19 的涂抹条（素材 694px 被声明为 5552px）

素材真值（`sips` + PIL 像素分析）：`walk-sheet.png` = **694×218**，8 帧、帧距 86.75px（列间隙在 x=86/174/262/350/437/520/607），墨迹仅占纵向 0..71 行。

`pet.html:10-11`：

```css
#dog-run.sheet{width:88px;height:72px;background-image:url("walk-sheet.png");
               background-size:5552px 72px;animation:walkcycle 1s steps(8) infinite}
@keyframes walkcycle{from{background-position:0 0}to{background-position:-5552px 0}}
```

`5552 = 694 × 8`：作者把"整张素材宽"当成了"单帧宽"。结果

- 横向 8× 拉伸（694→5552）、纵向 3× 压扁（218→72）；
- 88px 元素窗口每步只看到源图 88/8 = **11px**（单帧宽的 12.7%）；
- 逐帧量化：as-shipped 墨迹覆盖 5–15%、包围盒 88×18~21（长宽比 4.0–5.4）；按 1:1 渲染应为 55–57%、85~88×72（1.2）。

帧切分本身是对的（8 个位置各不相同），所以"在做动画"，但画出来的不是狗。提交 `228f3d2` 同时引入素材与此 CSS；此后的验收用的是"棕像素计数""帧差 px"（§59/§66/§67/§69），这类指标对涂抹条同样成立 —— 这是自验方法的漏洞，不是手误。

修（2 处）：`background-size:694px 218px` + `to{background-position:-694px 0}`（`steps(8)` 保留：694/8=86.75 = 恰好一帧）。

对照图：`sprite-as-shipped-vs-intended.png`（我按 CSS 语义离线渲染的 as-shipped / as-intended 两行对照，**非截屏**，用于 10 秒目视确认）。

### P0-3 App Nap 没有被禁用（"恍惚影子"的修复是空转）

`dock.rs:275-298`：

```rust
let activity: *mut c_void = msg_send![pi, beginActivityWithOptions: opts
                                     reason: cfstr as *mut AnyObject];
// 永久持有（不 end）——变量本身不存也 OK：activity 对象被系统保留在进程活动表
let _ = activity;
```

三条独立证据说明注释是错的：

1. Apple 头文件原文（`NSProcessInfo.h:117`）：*"If the object is deallocated before the `-endActivity:` call, the activity will be automatically ended."*
2. 我的 ObjC 探针：`beginActivityWithOptions:reason:` 返回的是 **+0（autoreleased）** 对象 —— 池内持有强引用时 weak 存活，显式释放后池 drain 即 dealloc（对照组 `[NSObject new]` 行为一致）。Tauri runloop 的 autorelease pool 一 drain，活动即结束。
3. 位掩码也错：`let opts = (0xFFu64<<20) | (1<<20) | (1<<0);`，而 SDK 实测 `NSActivityLatencyCritical = 0xFF00000000`（bit 32–39）、`NSActivityIdleDisplaySleepDisabled = 1<<40`。代码只对了 `1<<20`（IdleSystemSleepDisabled）；LatencyCritical 与 DisplaySleep 都没设上，`1<<0` 落在 `NSActivityBackground`(0x000000FF) 掩码里。

结论：§69 的"根因修复"不成立，"`[CostDog] app nap disabled`" 只是一行日志。

修：进程级持有 token（`static OnceLock<Retained<AnyObject>>` 或 `Retained::retain` 后 leak）+ 正确掩码 `(0xFFu64<<32)|(1u64<<40)|(1u64<<20)` + `CFRelease(cfstr)`。

补充（避免过度归因）：我**没有**测到宠物页当前被挂起。unified log 显示宠物页每 500ms 产生一次 custom-scheme IPC load（≈ `set_pet_rect` 的节拍），说明它的 JS 定时器是活的；被反复 `freezeLayerTree`/`markLayersVolatile`（140 轮/70min，退避 20→1280ms 全部失败重试）的是**隐藏窗口**那个 WebContent。也就是说：App Nap 保护现在是空转但暂时没有可观测后果；一旦宠物窗被判为不可见，同一机制就会落到它身上。这属于"根因未真正修掉、暂时被别的因素掩盖"，不是"已经从根上解决"。

### P0-4 CSP 拦掉了宠物页对 9401 的 fetch → 召唤自救是死的

- `pet.html:126` `fetch('http://127.0.0.1:9401/pet.json')`
- `tauri.conf.json:26` `connect-src 'self' ipc: http://ipc.localhost`
- Tauri 对每个 `.html` 资产都注入该 CSP（`tauri-2.11.3/src/manager/mod.rs:437-453`），跨源 `connect-src` 在 CORS 之前就被拒；服务器那行 `Access-Control-Allow-Origin: *` 帮不上。
- 本项目自己的历史已证明该 WebView 会执行 CSP：`3a2cbb3 "Tauri CSP blocks inline event handlers in production WebView"`。

运行期**决定性**证据：宠物页应以 2 次/秒访问 9401，而
`netstat -an -p tcp | grep .9401` 两次采样（间隔 4s）**只有 LISTEN 一条**，没有任何客户端连接；我用 1 次 `curl` 就能让采样出现第 2 条。若页面在正常 fetch，客户端连接/TIME_WAIT 会持续存在。

后果：`lastSummon` 恒为 -1，`dock::bump_summon()` 的前端消费端永不触发 —— 用户唯一的"找不到狗就点它"自救开关失效，而它正是 P0-1 的官方缓解措施。

修（推荐）：把召唤改成 IPC 命令（如 `get_summon_count`），保持 `connect-src` 收紧 —— 这也符合仓库自己的测试取向（`tests/tauri-config.test.ts:93` 明确禁止桌面端依赖 web 端口）。

---

## P1（显示/统计正确性）

### P1-1 zcode ▲速率永远只有"完成后 45s"，没有"生成中"

`lib.rs:3006-3008` 只取 `status IN ('completed','error','cancelled')`，而 `:3024` 的实时分支要求 `started_at + duration_ms > now`。

本地真值（`~/.zcode/cli/db/db.sqlite`，28791 行 completed、83 cancelled、45 error、**0 running**）：

- `SELECT COUNT(*) ... WHERE status='completed' AND started_at+duration_ms > completed_at` → **0**
- `MIN/MAX(completed_at-(started_at+duration_ms))` → **0 / 0**（即 `end == completed_at`，恒不晚于 now）

→ 实时分支不可达，实际语义是"该次响应完成后保留 45s 显示其真实 TPS，其余为 0"。文档 §5 的"生成中实时"不准确，而 §28/§29/§36 三次"速率不显示"的修复都没触及这一层 —— 这恰好解释了用户"跑着不显示速率"的反复投诉。

修：把 `status='running'` 纳入查询，running 行用 `output_tokens/((now-started_at)/1000)`（或 `COALESCE(duration_ms, now-started_at)`）。

### P1-2 缓存命中率：只修了 codex，claude-code/opencode 仍是错分母

`lib.rs:3141-3145` 只对 `source=="codex"` 补 `cache_read`，其余源直除；`:3138-3140` 的注释断言"zcode/claude 的 input 本身含缓存读"。实测（`~/.costdog/costdog.sqlite`，sessions 聚合）：

| source | Σinput | Σcache_read | cr/in（现算法） | cr/(in+cr)（正确语义） |
|---|---|---|---|---|
| claude-code | 365,151,708 | 5,230,608,645 | **1432.4%** | 93.5% |
| codex | 422,223,816 | 7,868,615,487 | 1863.6%（已走正确分支） | 94.9% |
| opencode | 1,960,529 | 23,609,749 | **1204.3%** | 92.3% |
| zcode | 7,254,686,793 | 6,816,909,936 | 94.0% ✅ | 48.4% |

claude 的 `usage.input_tokens` 是 Anthropic 语义的**未缓存输入**（`lib.rs:801` 与 `cache_read_input_tokens` 并列读取），所以注释对 claude 是错的、对 zcode 是对的（zcode 侧交叉验证：`provider_total_tokens == input+output` 恒成立，说明其 input 含缓存读）。

修：claude-code/opencode 归入 `+cache_read` 分支；更稳的做法是在 parser 写入处显式标注每个 source 的 input 语义，而不是在下游按源名硬编码。

### P1-3 面板窗口没有 capability → 事件订阅与版本号被 ACL 拒

`capabilities/default.json` `"windows": ["main"]`（生成的 `gen/schemas/capabilities.json` 同），而面板窗口 label 是 `panel`（`lib.rs:3465` 加载 `index.html#panel`），`index.html:919-923`（`tauriListen` 内的 `plugin:event|listen`）与 `:955` 的 `plugin:app|version` 会被按窗口 label 拒绝，且 `tauriListen` 没有 `.catch`。

影响：面板收不到 `refresh-data`/`update-*` 推送（`:1156` 的 5s 轮询兜住了，所以非致命）；版本号标签空白。`pet.html` 不受影响 —— app 自定义命令不参与 ACL（`acl-manifests.json` 无 `app` 键），实测 topbar 窗口 invoke 正常。

---

## P2（健壮性/卫生/文档）

1. **光标守卫线程起了两份**：`lib.rs:3364`（`ensure_topbar_window` 内）与 `lib.rs:3700`（setup 内）都会 spawn，`spawn_cursor_guard` 无 `Once` 保护；每线程各自维护本地 `pass_through`（`dock.rs:228`），过渡态可能互相打反，表现为偶发"第一下点不中"。`set_ignore_cursor_events(true)` 也重复（`3338`/`3367`）。
2. **NSPoint 的 Encode 写错**：`dock.rs:94-97` 用 `Encoding::Struct("NSPoint", ...)`，真机为 `{CGPoint=dd}`（我编译探针确认 `@encode(NSPoint)` 与 `method_copyReturnType(mouseLocation)` 都是 `{CGPoint=dd}`）。objc2 在 `debug_assertions` 下校验返回编码 → **debug 构建（`tauri dev`/`cargo test` 跑 app）里守卫线程首次调用即 panic 并静默死掉**（宠物永远不可点）；release 只是跳过校验。`dock.rs:99-102` 的 `RefEncode` 是死代码。
3. **走路时不会转向**：`pet.html:16` 的 `stepbob` 动画占用 `transform`，CSS 动画优先级高于行内 style ⇒ `:74` 的 `scaleX(-1)` 在 walking 态被丢弃（子代理 headless Chrome 实测计算值为 `matrix(1,0,0,1,0,0)`）；只有停下（无动画）时才翻转。§65 的"镜像修复"只覆盖静止态。
4. **走路态没有白描边**：`pet.html:13` 是 `img.sprite{filter:...}`，而走路精灵是 `div#dog-run`（`:40`）⇒ §57 的"任何背景可见"在最需要它的走路态失效。
5. **`full_scan()` 在 setup 主线程同步执行**（`lib.rs:3709`），且发生在宠物窗创建（`:3699`）之后 → 冷启动首帧渲染被阻塞，扫描耗时随数据量增长，形状就是"启动后一段时间看不见宠物"。
6. **9401 被占用时静默降级**：`lib.rs:3225-3228` bind 失败只打一行日志就 return，无重试无提示；且无单实例保护 ⇒ 双开时第二个实例没有 `/pet.json`，而验收脚本会读到第一个实例（旧坐标）→ **假验收**。
7. **HTTP 细节**：未知路径返回 200 + "not found"（`lib.rs:3254`，应 404）；CORS `*` + 无鉴权 ⇒ 用户访问的任意网页 JS 都能读 `/stats.json`（项目名 + 今日花费）。绑定本身没问题（`lsof` 实测 `127.0.0.1:9401 LISTEN`）。
8. **BAR_VISIBLE 语义已与新形态脱节**：初值 true、只有 `hide_bar` 置 false（`lib.rs:235/3186`），而 `show_bar` 现在只弹面板（不再显示主条）。当前侥幸每 30s 扫描；一旦有人触发 `close_window`，扫描会永久降到 5 分钟节拍，而宠物仍在屏上、数字悄悄变旧。
9. **reinforce 看门狗无条件写**：`dock.rs:312-318` 每 3s 重写 `styleMask`/`collectionBehavior`（`behavior` 用赋值而非 OR，会丢掉 AppKit/用户后加的位），且 `:330` `let _ = app.run_on_main_thread(...)` 吞掉 Err —— 主循环退出后看门狗静默失效，正是它要防的场景。建议改为"缺位才写" + 失败打日志并退出循环。
10. **`SCREEN_H` 只在启动写一次**（`dock.rs:17/107`，`set_screen_geo` 仅由 `lib.rs:3346` 调用），无显示器变更处理 → 换屏/改分辨率后光标命中判定错位（宠物点不中或抢点击）。
11. **死代码/死资产**：`spawn_roamer`（内含 `set_position`，与自家铁律 1 相悖）/`ROAM_*`/`get_walk_state`（pet.html 已不轮询）/`index.html` 的 `#pet`+`#topbar` 整套（`IS_TOPBAR=false` 硬编码、`IS_PET` 分支因宠物窗加载 pet.html 而不可达）/`embed_*` 注入链。退役素材 9.6MB 中仅 ~0.33MB 被引用（`walk-sheet.png`+`pet-sleep.png`），`cat.png`/`rabbit.png`/`sprite.png`/`walk-0..7`/`walk-raw.png`/`pet-cat-*`/`pet-rabbit-*` 全无引用（cat/rabbit 端点也无人调用）。
12. **`tauri.conf.json` 唯一的窗口是退役主条**：无 `visible:false`（先可见后 `hide()`）、`alwaysOnTop:true` 与 `lib.rs:3672` 的 `set_always_on_top(false)` 直接矛盾、title 被 `pet.html:142` 每 400ms 改写成 `PET:x,y,w`；真实三窗（main/panel/topbar）全在 Rust 里现建。
13. **测试现状**：`npm run test:ts` **现在是红的** —— `tests/tauri-config.test.ts:71-73` 断言 index.html 无 CJK，实测 947 处（pet.html 另有 146 处且不受管）；本环境无 `node_modules`，`tsx` 跑不起来。**没有一条测试读 `pet.html`**，P0-1/P0-2/P0-4 全在盲区。另 `package.json` 把 `tsx`/`typescript`/`@types/*` 放在 `dependencies`（发布 CLI 会拖入构建依赖）。
14. **文档漂移（review 交接会被误导）**：
    - §6.6 "149 提交未 push" → 实际 `origin/main..main = 64`（总提交 150）。
    - §3 "⇄ 切换三只宠物" → `pet.html:48-49` 只有 `ANI=['dog']` 和一个 `stopPropagation` 的点击处理，**无切换逻辑**；§6.2"猫/兔是单帧平移"实为**不可达**。
    - §8.1 声称顺序"App Nap 禁用→数据服务→宠物窗→守卫线程"，代码实际是 panel→App Nap(空转)→宠物窗(内部起守卫)→重复起守卫→数据服务→托盘→`full_scan`。
    - §4.4/§5/§69 的三条"已修复"分别被 P0-3/P1-1(+P1-2)/P0-3 推翻。
    - `AGENTS.md` 仍把 410×36 bar + 5 皮肤 + "UI 全英文（有测试守）"当作现状，与宠物形态冲突。

---

## 交接 checklist 逐条回答（HANDOFF §8）

1. **setup 顺序与窗口生命周期竞态**：顺序见上（文档不符）。真实问题：App Nap 空转（P0-3）；宠物窗先建、`full_scan()` 后同步阻塞主线程（P2-5）；光标守卫重复（P2-1）；数据服务 bind 失败静默（P2-6）。窗口生命周期本身没发现新的竞态，`make_non_activating` 都在主线程。
2. **dock.rs 手写 FFI**：数值常量**全部正确**（`NSNonactivatingPanelMask=1<<7`、`NSStatusWindowLevel=25`、`CanJoinAllSpaces=1<<0`、`FullScreenAuxiliary=1<<8`），AppKit 调用**都在主线程或 `run_on_main_thread` 内**，`run_on_main_thread` 是非阻塞投递不会死锁 —— 这几点可以放心。但 `NSPoint` 编码错（debug 崩守卫线程，P2-2）、App Nap token 丢弃 + 掩码错（P0-3）、CFString 泄漏（P2-9 附带）、`SCREEN_H` 不更新（P2-10）、`unwrap` 于后台线程（`dock.rs:666`）。
3. **pet.html 铁律核对**：无 `img.src` 换帧 ✅（R1 形式成立，但 P0-2 的尺寸是另一个缺陷）；`#stage` 是 `fixed` 底条、无 `inset:0` 全铺 ✅（R2）；`clampX(x)=max(10,min(SW-160,x))` 存在且每 110ms 刷新 ✅（R3 成立，但余量只有 2px：铭牌实测最宽 166px、居中于 150px 容器 ⇒ 右夹取处 plateRight=1918/1920；建议按 `plate.offsetWidth/2+2` 夹取）；窗口从不被程序移动 ✅（R4，唯一可达的 `set_position` 是菜单栏面板）。另外 R1 反而暴露了两个新 bug（P2-3 转向、P2-4 描边）。
4. **9401 是否只绑回环**：✅ `lsof` 实测 `127.0.0.1:9401 (LISTEN)`，无 0.0.0.0。CORS `*` 的实际风险是"任意网页可读本机花费/项目名"（P2-7），且未知路径返回 200。
5. **CLAUDE.md §5-69 与 git log 对照**：每个"功能"基本都有对应提交（`git log --oneline | sed -n '30,80p'` 与 §28-§41 一一对应），但**"已修复"的结论有 4 条站不住**（P0-3、P1-1、P1-2，以及 §65 只覆盖一半状态的镜像/描边修复）。也就是说：提交存在、验证记录存在，但验证方法（棕像素计数、帧差 px、窗口级截图）不足以支撑结论。

---

## 我验证通过的部分（可以当作已确认的资产）

- 透明窗三件套齐备：`tauri.conf.json:12 macOSPrivateApi:true` + `Cargo.toml:14 features=["tray-icon","macos-private-api"]` + builder `.transparent(true)`。
- 宠物窗确实从不被移动；走路帧动画是 CSS 雪碧图（无 `img.src` 换帧）；`#stage` 为 fixed 底条（铁律 1/2/8 的形式都守住了）。
- `cargo test`：**38 passed / 0 failed**（实跑）。内联脚本 `node --check` 全部通过。
- 所有 `setInterval`/`fetch`/`invoke` 都有 `.catch`，不会卡死动画循环，无可达 NaN 路径。
- `lsof` 只回环监听；`unified log` 证明宠物页 JS/IPC 是活的（500ms 节拍）。

## 我无法验证的部分（请在流程上补）

- **任何屏幕截图**：本进程 `screencapture` 报 `could not create image from display`（缺屏幕录制 TCC）；`osascript`/System Events 也超时（缺辅助功能权限）。所以 HANDOFF §7/§4.9 的"自主验收闭环"对 review agent **不可复现**，除非给运行 agent 的宿主 App 授权屏幕录制。
- 建议把"亲眼看图"这一步换成不需要 TCC 的断言：headless Chrome 的 computed style（已验证可行：子代理用 `--headless=new` 拿到了 `background-size`、`display`、`matrix()` 的真值）+ PIL 素材断言（帧数/帧距/墨迹高度）+ DOM/CSS 断言。这样 P0-1/P0-2/P2-3/P2-4 四类问题都能被 CI 挡住，而不是依赖人手截图。

---

## 建议的修复顺序

1. `pet.html:41` 删 `hidden` + `:30` 规则（1 行，解决"狗不见了"的主因）。
2. `pet.html:10/11` 改 `694px`（2 处，解决"涂抹条"，用附带对照图核对）。
3. `dock.rs` App Nap：持有 token + 正确掩码（否则 §69 的结论一直是假的）。
4. summon 走 IPC，去掉对 9401 的页面内依赖（恢复用户的自救开关）。
5. zcode 速率纳入 `status='running'`。
6. 命中率分母按 parser 写入语义逐源声明（claude-code/opencode 归入 `+cache_read`）。
7. `capabilities` 加 `"panel"`；`tauriListen` 补 `.catch`。
8. 光标守卫去重（删一处 spawn）；`NSPoint` 改 `{CGPoint=dd}`。
9. 转向与 bob 解耦（bob 移到内层元素）；`img.sprite` → `.sprite` 修描边。
10. 清死代码/死资产（约 9.3MB）+ 修 `AGENTS.md`/`tauri.conf.json`/`HANDOFF.md` 的漂移；把 `pet.html` 纳入测试并把 CJK 断言收窄到文本节点（当前它是红的）。

> 顺带纠正 §6.6 的 push 策略依据：未 push 的是 64 个提交而非 149。真正值得先做的是让 CI 跑起来（TS 测试现在是红的），否则 push 上去 CI 也会红。
