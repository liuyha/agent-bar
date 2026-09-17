# AgentBar 架构

React 展示账号与用量，Rust 读取本机登录态并采集真实数据。运行时没有演示 Provider；未登录、网络错误和不支持的登录模式都有独立状态，不生成占位百分比。

## 模块边界

| 模块 | 职责 |
| --- | --- |
| `src/lib/api.ts` | 封装 Tauri commands/events；浏览器仅提供“请使用桌面端”状态和本地设置 |
| `src/types/index.ts`、`src-tauri/src/models.rs` | camelCase JSON 契约 |
| `src-tauri/src/providers/codex.rs`、`providers/codex/` | 本机 PAT / OAuth 账号额度、身份隔离与受控 app-server 回退 |
| `src-tauri/src/providers/claude.rs` | 读取 Claude Code 本机登录态，查询官方订阅用量 |
| `src-tauri/src/providers.rs` | 并行采集启用的服务、隔离失败 |
| `src-tauri/src/state.rs` | 设置与用量快照持久化、快照版本与并发控制 |
| `src-tauri/src/storage.rs` | 用户数据目录、私有文件权限与 JSON 原子读写 |
| `src-tauri/src/lib.rs` | Tauri 命令、后台任务、托盘事件 |
| `src-tauri/src/tray_summary.rs` | 菜单栏剩余占比、重置倒计时与来源提示 |
| `src-tauri/src/panel.rs` | 主面板与附属统计窗口定位、级联交互、焦点组与显示版本 |
| `src-tauri/src/statistics.rs` | 读取本地会话、去重、按本地日期统计 Token / 请求 / 轮次 |
| `src-tauri/src/statistics/codex_history.rs`、`statistics/cache.rs` | Codex / pi / OMP 日志增量解析、继承记录处理，以及 Codex / Claude SQLite 缓存 |
| `src-tauri/src/statistics/pricing.rs` | 精确模型价格映射、缓存和长上下文计价 |
| `src/components/TokenStatisticsPanel.tsx` | 悬停详情与统计加载、缺失、错误状态 |
| `src/components/StatisticsWindow.tsx` | 独立统计窗口入口、账号/设置/选中服务同步与渲染确认 |
| `src/lib/panel.ts` | 串行提交面板操作、订阅状态与携带版本确认显示 |
| `src/lib/useAccountStatistics.ts` | 服务端统计 store 的初始化、定时刷新、手动事件与账号生命周期 |
| `src/lib/tokenStatistics.ts` | 按服务保留最新统计、读取已保存汇总、合并并发请求并后台更新 |
| `src-tauri/src/account_statistics.rs`、`providers/codex/activity.rs` | 账号服务端活动契约、OAuth / PAT / CLI 来源与字段归一化 |
| `src-tauri/src/codex_web.rs`、`codex_web/` | 用户主动连接的独立网页会话、账号核对及可选网页指标 |
| `src/components/AccountStatisticsContent.tsx`、`src/lib/accountStatistics.ts` | 服务端统计视图、请求版本和来源隔离 |

## 数据契约

`ProviderUsage` 包含 `id`、`name`、`plan`、`account`、`source: "local"`、`status`、`message`、`windows` 和 `updatedAt`。

- `status` 为 `ready`、`unavailable` 或 `error`。登录缺失、API key 等没有订阅限额的模式不冒充成功。
- `account`、`message` 和 `updatedAt` 可为 `null`。没有成功读到用量时不显示成功采集时间。
- 每个窗口包含 `label`、`usedPercent` 和可空的 `resetsAt`。只显示服务实际返回的窗口；窗口数量和周期不固定。
- `DashboardSnapshot` 包含 `providers`、采集尝试的 `updatedAt`、`mode: "live"` 和递增的 `revision`。重启恢复已保存的版本后继续递增；单个服务失败不阻断其他服务。

`AppSettings` 包含 `enabledProviders`、`refreshIntervalSeconds`、`theme`：默认两项服务、300 秒、跟随系统；支持 60/300/900 秒刷新和 system/light/dark 主题。新增 `codexStatisticsSource: local | auto | oauth | pat | cli`（默认 local）及 `codexWebExtras`（默认 false）。旧设置缺少新字段时按默认值恢复，网页连接始终由用户主动发起。

| Command | 作用 |
| --- | --- |
| `get_dashboard` | 返回从磁盘恢复或最近持久化成功的内存快照 |
| `refresh_dashboard` | 在线程池读取账号与用量，持久化并读回快照后广播 |
| `refresh_provider_dashboard` | 仅刷新指定服务，保留其他服务后持久化并广播 |
| `get_settings` | 读取本机设置 |
| `save_settings` | 持久化设置并立即应用服务过滤，通知后台重新采集 |
| `hide_panel` | 收起主面板和统计窗口，后台继续运行 |
| `hide_settings` | 隐藏独立偏好设置窗口，不影响主面板 |
| `show_statistics_panel` | 主窗口按 provider、卡片完整可见矩形和聚焦意图请求展开独立统计窗口 |
| `hide_statistics_panel` | 收起统计窗口，可将焦点交回主窗口 |
| `dismiss_panel` | 按层级关闭：先统计窗口，再主面板 |
| `set_panel_interaction` | 汇报调用窗口的悬停与键盘交互状态，协调跨窗口延迟收起 |
| `get_statistics_panel_state` | 读取选中 provider、左右方向与递增 revision |
| `present_statistics_panel` | 统计窗口完成渲染后回传 revision，原生校验仍有效才显示 |
| `get_token_statistics` | 在后台线程同步指定服务源日志、聚合统计，写入汇总 JSON 后读回返回 |
| `get_cached_token_statistics` | 只读指定服务已保存的汇总 JSON；缺失时返回 null，不扫描日志 |
| `get_codex_account_statistics` | 自动选择服务端认证和查询方式，按已保存开关附加网页数据；返回前复核来源、账号及设置 |
| `get_cached_codex_account_statistics` | 仅读取当前登录及配置范围匹配的服务端成功缓存，不发起采集；缺失或失效返回 null |
| `open_codex_usage_web` | 开关启用后，打开独立的 ChatGPT 用量网页登录窗口 |

`usage-updated` 广播快照；`settings-updated` 广播保存后的设置，使主面板同步主题与服务过滤；`navigate-usage` 重置主面板导航。偏好设置由托盘右键菜单打开独立的 `settings` 窗口（`index.html#settings`），关闭时隐藏并保留草稿，不随主面板失焦收起。前端拒绝较低 `revision`，防止命令返回与后台事件乱序覆盖。

`statistics-panel-changed` 向主面板和统计窗口发送 `{ provider, side, revision }`。统计窗口先订阅再读当前状态，完成对应版本渲染后调用 `present_statistics_panel`；Rust 仅接受仍选中服务、主面板仍可见且 revision 一致的确认，避免快速切换或关闭后的迟到渲染重新弹窗。`refresh-token-statistics` 携带 provider，使卡片手动刷新即使返回同样的不可用额度状态，也能触发本机统计更新；`refresh-account-statistics` 将账号统计手动刷新交给唯一的服务端 store。

## 采集与并发

启动从 `~/.agent-bar/dashboard.json` 恢复用量快照，按当前设置过滤服务并为新启用服务补充待采集状态，然后建立托盘；真实采集在后台继续执行。凭据读取、CLI 和 HTTP 不占用 UI 线程。两个服务并行采集，单服务错误转成对应卡片。刷新互斥防止同一时刻重复访问账号，采集期间不持有设置/快照锁。有效刷新结果先原子写入并读回，再替换内存快照和广播；存储失败返回错误并保留原内存值。

设置保存不会等待网络请求；先原子落盘，再更新内存并递增快照版本。正在执行的旧设置采集结果会被丢弃，后台按新设置继续采集。后台首次启动、设置变更、定时器到期以及手动刷新都会触发真实查询。

## 账号边界

Codex 额度按可用的 PAT → OAuth → 受控 CLI 路径读取。Rust 读取 `CODEX_HOME` 下的 `auth.json`，OAuth 使用 `https://chatgpt.com/backend-api/wham/usage`；PAT 先读取 `whoami` 确认账号再请求用量。账号 ID 随当前凭据传递，不从其他配置目录借用身份。网络、服务端错误及限流不会转成额外 CLI 请求；可恢复的原生认证问题交给同一配置范围的官方 CLI。自定义 backend 也由 CLI 处理。

CLI 路径继续使用 `account/read` 与 `account/rateLimits/read`，不启动模型任务。账号和限额解析支持多 bucket、可空重置时间、API key 模式和未登录状态。AgentBar 自己不写入共享凭据文件，原始 token 和上游错误正文不会传到前端。协议依据 [Codex App Server 官方文档](https://learn.chatgpt.com/docs/app-server)；内部 HTTP 契约及具体对齐边界见 [Codex 对齐说明](codex-alignment.md)。

Claude 使用 Claude Code 的本机 OAuth 登录来源；只有凭据读取和官方 HTTPS 请求位于 Rust，前端只能拿到身份元信息、用量和脱敏错误。API key/代理模式不支持 Claude 订阅用量。来源依据 [Claude Code 认证文档](https://code.claude.com/docs/en/authentication)和官方 CLI 的账户接口实现。HTTP 限时、禁止重定向，不将 token 写入设置或日志；AgentBar 不替用户登录或刷新 Claude 凭据。

普通浏览器不能读取本机账号，页面明确提示通过 `pnpm desktop:dev` 或打包应用运行，不添加本地 HTTP 凭据服务。

## 本地历史统计

历史统计独立于账号额度快照。悬停服务卡片或用鼠标／键盘激活统计入口加载相应服务；服务展示数据变化或卡片手动刷新后重新统计。桌面详情在常驻的独立统计窗口中展示，前端按服务保留最新结果，关闭详情不会丢弃缓存或进行中的请求，再次展开立即显示上次结果并后台更新。同一服务的并发刷新共用请求，两个服务的缓存与订阅互相隔离。只有没有缓存的首次加载展示全屏统计提示；更新失败保留旧结果并提示重试。原生展开、收起和交互操作在各窗口前端串行提交，跨窗口状态由 Rust 协调；浏览器预览继续采用页内详情。

`TokenStatistics` 包含 `status`、可空 `message`、`updatedAt` 与 `periods`。每个周期包含 `period: day | week | month | year | all`、`startAt` / `endAt`、`inputTokens`（包含缓存）、`cachedInputTokens`、`cacheWriteTokens`、`outputTokens`、`totalTokens`、可空 `estimatedCostUsd`、`unpricedTokens`、可空 `requestCount` / `conversationTurns`。按本机日历零点分界，本周从周一开始，本年从当年 1 月 1 日开始；全部涵盖本机保留的所有记录，截至读取时刻，起点取最早有效记录时间，没有记录时使用结束时间。全部统计不按月裁剪解析结果。缺失或不能可靠重建的计数保持 null，与零区分。

Codex 原生历史读取 `sessions` 和 `archived_sessions`，同时解析 pi / OMP 的 `openai-codex` assistant usage；Claude 读取 `projects`。源目录继续尊重 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。模型标记、原生请求记录和累计快照分别处理；分叉或子智能体继承的记录不重复计入。两个服务分别将归一用量保存至 `~/.agent-bar/{codex,claude}-token-history.sqlite3`，重启后复用缓存；Codex 还保存文件身份和解析检查点，继续读取新增日志，未完整写入的行留待后续追加。

没有前端内存缓存时，先通过只读命令加载 `{codex,claude}-token-statistics.json`，无需等待日志扫描。每次展开或刷新仍在后台同步源日志变化并按当前本地日历聚合，随后将结果保存至统计 JSON 并读回更新前端缓存；这期间保留已有结果和真实统计时间。已有汇总不会阻止采集新增数据或重新计算日期范围。

会话只在 Rust 中解析，不把对话正文或原始日志发送到前端。模型金额采用核验的公开标准单价，无价格的模型保留 Token 统计；`estimatedCostUsd` 汇总可计价部分，并通过 `unpricedTokens` 提示不完整，全部无法计价时为 null。费率来源与边界记录在 [计价依据](token-pricing.md)。本地数据可能跨账号，不能与当前账号额度比例混为一谈。

## 账号服务端统计

`AccountUsageSnapshot` 与本地 `TokenStatistics` 分离，包含实际来源、账号元信息、可空活动汇总、可空每日记录、服务端日期和采集时间。界面只展示“本机记录”和“服务端”，服务端内部自动选择 PAT / OAuth / 受控 CLI 路径，不展示实际认证方式。旧版显式来源设置迁移为 `auto`。OAuth / PAT 直接请求 `wham/profiles/me`；CLI 使用 `account/usage/read`。前端 `accountStatisticsPeriods.ts` 按本地日历选择服务端日期，今日／本周／本月／本年仅汇总匹配的每日记录；全部只读服务端累计与峰值汇总，缺失不回退为有限日记录总和。两类来源共用 `StatisticsPeriodSwitch`，时段切换不触发额外请求；账号活动概览与网页补充不随时段变化。网页补充采用独立的临时登录会话，仅允许已确认的同账号数据。详细字段和单位边界见 [账号服务端统计](account-statistics.md)。

`account_statistics_cache.rs` 保存带版本、来源和登录范围摘要的成功快照，复用原子写与私有权限；请求顺序校验防止迟到结果覆盖新请求。只读缓存命令不调用采集器，查询及保存前后复核当前范围和设置。桌面服务端 store 与定时器由常驻的 `StatisticsWindow` 通过 `useAccountStatistics` 唯一持有，主窗口和设置窗口不重复请求。初始化先仅读缓存、缺失再获取；隐藏统计窗口不卸载刷新生命周期。独立周期触发 `refreshing`（保留内容、按钮旋转），按钮与托盘手动事件触发 `loading`（面板加载）；在途请求合并，手动可升级自动请求的反馈。账号/来源/网页设置变化会取消旧生命周期并重新核验缓存。

服务端结果不写入本机汇总 JSON，不复用仅按 ProviderId 区分的本机缓存。前端每次请求递增版本，来源／账号／网页开关改变后取消旧结果；认证失败清除旧远端值。Rust 在采集前后复核已保存设置以及当前 `CODEX_HOME` 的认证／配置指纹，防止异步请求返回另一账号的数据。网页本身无 Tauri 原生权限；账号统计和网页连接命令仅允许可信的 `main`、`settings`、`statistics` 窗口。面板控制限于 `main` / `statistics`，展开请求仅来自 `main`，渲染确认仅来自 `statistics`。

## 菜单栏摘要

macOS 托盘标题格式为 `<剩余占比> <距离重置时长>`，例如 `60% 2天 20小时`。优先选启用且读取成功的 Codex 第一个有效额度窗口（核心窗口在前），不可用时选 Claude；剩余比例为 `100 - usedPercent`，限制在 0–100 后取整数。提示文字说明账号服务及具体窗口。没有有效用量时清空摘要，避免继续显示旧数值；重置时间未知或已到期时明确标识。

首次采集、手动/后台刷新和设置保存都会同步摘要；独立的 30 秒计时器仅从内存重算倒计时，不增加网络查询。托盘更新在主线程读取最新快照，防止旧事件将已关闭的服务重新显示。Tauri 的 Windows 托盘不支持标题，Linux 标题能否显示由桌面环境决定；当前验证目标为 macOS。

## 桌面与存储

主面板默认隐藏，左键托盘图标展开 360×600 逻辑像素面板，统计展开不改变主面板尺寸。启动时预创建隐藏的附属 `statistics` 窗口（`index.html#statistics`），关闭时隐藏并复用；macOS 以原生父子窗口关联。统计窗口优先贴主面板右侧，右侧放不下时改为左侧，纵向按服务卡片锚点定位并限制在屏幕工作区内。两侧都不足时，剩余空间达到 280 逻辑像素便缩窄详情；否则在工作区内覆盖展示，避免窗口超出屏幕。

当前服务卡片与可见统计窗口共同构成鼠标交互区域，主面板其余区域不保留详情。原生层在统计窗口展开期间读取真实屏幕鼠标位置，连续位于交互区域外约 250 ms 才收起；关闭前再次核对坐标和当前选择版本，不以 WebView 的 `:hover` 或 enter／leave 到达顺序决定关闭。卡片与明细纵向重叠范围内的窄水平间隙也作为过渡区，允许慢速跨窗移动；卡片下方空白不在范围内。前端在滚动和卡片尺寸改变时同步完整可见矩形。纯键盘导航可以保留面板，实际鼠标移动后恢复位置判断，旧 DOM 焦点不能阻止收起。自动收起已聚焦的明细时还焦主面板；点击外部仍按两窗焦点关闭整组。悬停打开不抢焦点；点击统计入口或按展开方向键将焦点交给统计窗口，反方向键可返回主面板。Esc 先关闭详情，再关闭主面板；再次点击托盘直接隐藏整组。

账号额度的 Rust 后台采集独立于 WebView；服务端活动统计由常驻统计 WebView 管理。macOS 14 及以上禁用统计 WebView 后台节流，使隐藏期间仍可处理刷新和渲染确认；其他平台及系统版本的隐藏执行时机受 WebView 能力影响。macOS 使用 NSStatusItem 屏幕坐标定位；Linux 通过托盘菜单打开。Windows、Linux 的原生行为仍需独立验收。

统计数据统一保存在用户主目录的 `.agent-bar/`。`dashboard.json` 保存用于展示的账号元信息、套餐、额度和更新时间；两个服务各自的 SQLite 保存归一历史，统计 JSON 保存聚合结果。这里不保存账号凭据、额度接口原始响应或对话正文。macOS / Unix 目录权限为 `0700`，JSON 和 SQLite 文件权限为 `0600`。旧 Tauri 应用缓存保持原样，首次使用新目录时从源日志重建统计缓存。

设置继续保存在 Tauri 应用配置目录 `settings.json`；macOS 当前为 `~/Library/Application Support/dev.agentbar.desktop/settings.json`。浏览器设置保存在 `agentbar.settings.v1`，与桌面不互通。

开机启动、自动更新、应用签名和分发尚未接入。Codex 采集与历史统计参照 CodexBar，以 Rust 实现，未分发其 Swift 源码或 CLI；参照版本和 MIT 声明见 [第三方声明](third-party-notices.md)。AgentBar 的开源许可证尚未确定。
