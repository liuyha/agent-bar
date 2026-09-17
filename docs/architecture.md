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
| `src-tauri/src/panel.rs` | 面板定位、失焦收起、托盘点击切换 |
| `src-tauri/src/statistics.rs` | 读取本地会话、去重、按本地日期统计 Token / 请求 / 轮次 |
| `src-tauri/src/statistics/codex_history.rs`、`statistics/cache.rs` | Codex / pi / OMP 日志增量解析、继承记录处理，以及 Codex / Claude SQLite 缓存 |
| `src-tauri/src/statistics/pricing.rs` | 精确模型价格映射、缓存和长上下文计价 |
| `src/components/TokenStatisticsPanel.tsx` | 悬停详情与统计加载、缺失、错误状态 |
| `src/lib/tokenStatistics.ts` | 按服务保留最新统计、读取已保存汇总、合并并发请求并后台更新 |

## 数据契约

`ProviderUsage` 包含 `id`、`name`、`plan`、`account`、`source: "local"`、`status`、`message`、`windows` 和 `updatedAt`。

- `status` 为 `ready`、`unavailable` 或 `error`。登录缺失、API key 等没有订阅限额的模式不冒充成功。
- `account`、`message` 和 `updatedAt` 可为 `null`。没有成功读到用量时不显示成功采集时间。
- 每个窗口包含 `label`、`usedPercent` 和可空的 `resetsAt`。只显示服务实际返回的窗口；窗口数量和周期不固定。
- `DashboardSnapshot` 包含 `providers`、采集尝试的 `updatedAt`、`mode: "live"` 和递增的 `revision`。重启恢复已保存的版本后继续递增；单个服务失败不阻断其他服务。

`AppSettings` 保持 `enabledProviders`、`refreshIntervalSeconds`、`theme`：默认两项服务、300 秒、跟随系统；支持 60/300/900 秒刷新和 system/light/dark 主题。

| Command | 作用 |
| --- | --- |
| `get_dashboard` | 返回从磁盘恢复或最近持久化成功的内存快照 |
| `refresh_dashboard` | 在线程池读取账号与用量，持久化并读回快照后广播 |
| `refresh_provider_dashboard` | 仅刷新指定服务，保留其他服务后持久化并广播 |
| `get_settings` | 读取本机设置 |
| `save_settings` | 持久化设置并立即应用服务过滤，通知后台重新采集 |
| `hide_panel` | 收起面板，后台继续运行 |
| `hide_settings` | 隐藏独立偏好设置窗口，不影响主面板 |
| `set_panel_expanded` | 在屏幕工作区内展开 / 恢复统计窗口宽度 |
| `get_token_statistics` | 在后台线程同步指定服务源日志、聚合统计，写入汇总 JSON 后读回返回 |
| `get_cached_token_statistics` | 只读指定服务已保存的汇总 JSON；缺失时返回 null，不扫描日志 |

`usage-updated` 广播快照；`settings-updated` 广播保存后的设置，使主面板同步主题与服务过滤；`navigate-usage` 重置主面板导航。偏好设置由托盘右键菜单打开独立的 `settings` 窗口（`index.html#settings`），关闭时隐藏并保留草稿，不随主面板失焦收起。前端拒绝较低 `revision`，防止命令返回与后台事件乱序覆盖。

## 采集与并发

启动从 `~/.agent-bar/dashboard.json` 恢复用量快照，按当前设置过滤服务并为新启用服务补充待采集状态，然后建立托盘；真实采集在后台继续执行。凭据读取、CLI 和 HTTP 不占用 UI 线程。两个服务并行采集，单服务错误转成对应卡片。刷新互斥防止同一时刻重复访问账号，采集期间不持有设置/快照锁。有效刷新结果先原子写入并读回，再替换内存快照和广播；存储失败返回错误并保留原内存值。

设置保存不会等待网络请求；先原子落盘，再更新内存并递增快照版本。正在执行的旧设置采集结果会被丢弃，后台按新设置继续采集。后台首次启动、设置变更、定时器到期以及手动刷新都会触发真实查询。

## 账号边界

Codex 额度按可用的 PAT → OAuth → 受控 CLI 路径读取。Rust 读取 `CODEX_HOME` 下的 `auth.json`，OAuth 使用 `https://chatgpt.com/backend-api/wham/usage`；PAT 先读取 `whoami` 确认账号再请求用量。账号 ID 随当前凭据传递，不从其他配置目录借用身份。网络、服务端错误及限流不会转成额外 CLI 请求；可恢复的原生认证问题交给同一配置范围的官方 CLI。自定义 backend 也由 CLI 处理。

CLI 路径继续使用 `account/read` 与 `account/rateLimits/read`，不启动模型任务。账号和限额解析支持多 bucket、可空重置时间、API key 模式和未登录状态。AgentBar 自己不写入共享凭据文件，原始 token 和上游错误正文不会传到前端。协议依据 [Codex App Server 官方文档](https://learn.chatgpt.com/docs/app-server)；内部 HTTP 契约及具体对齐边界见 [Codex 对齐说明](codex-alignment.md)。

Claude 使用 Claude Code 的本机 OAuth 登录来源；只有凭据读取和官方 HTTPS 请求位于 Rust，前端只能拿到身份元信息、用量和脱敏错误。API key/代理模式不支持 Claude 订阅用量。来源依据 [Claude Code 认证文档](https://code.claude.com/docs/en/authentication)和官方 CLI 的账户接口实现。HTTP 限时、禁止重定向，不将 token 写入设置或日志；AgentBar 不替用户登录或刷新 Claude 凭据。

普通浏览器不能读取本机账号，页面明确提示通过 `pnpm desktop:dev` 或打包应用运行，不添加本地 HTTP 凭据服务。

## 本地历史统计

历史统计独立于账号额度快照。悬停、键盘聚焦或点击服务卡片加载相应服务；服务展示数据变化或卡片手动刷新后重新统计。前端按服务保留最新结果，关闭详情不会丢弃缓存或进行中的请求，再次展开立即显示上次结果并后台更新。同一服务的并发刷新共用请求，两个服务的缓存与订阅互相隔离。只有没有缓存的首次加载展示全屏统计提示；更新失败保留旧结果并提示重试。原生展开 / 收起命令在前端串行处理。卡片与右侧详情共享悬停区域，延迟判断移出，避免扩展窗口时的临时鼠标离开造成反复开合。

`TokenStatistics` 包含 `status`、可空 `message`、`updatedAt` 与 `periods`。每个周期包含 `period: day | week | month | year | all`、`startAt` / `endAt`、`inputTokens`（包含缓存）、`cachedInputTokens`、`cacheWriteTokens`、`outputTokens`、`totalTokens`、可空 `estimatedCostUsd`、`unpricedTokens`、可空 `requestCount` / `conversationTurns`。按本机日历零点分界，本周从周一开始，本年从当年 1 月 1 日开始；全部涵盖本机保留的所有记录，截至读取时刻，起点取最早有效记录时间，没有记录时使用结束时间。全部统计不按月裁剪解析结果。缺失或不能可靠重建的计数保持 null，与零区分。

Codex 原生历史读取 `sessions` 和 `archived_sessions`，同时解析 pi / OMP 的 `openai-codex` assistant usage；Claude 读取 `projects`。源目录继续尊重 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。模型标记、原生请求记录和累计快照分别处理；分叉或子智能体继承的记录不重复计入。两个服务分别将归一用量保存至 `~/.agent-bar/{codex,claude}-token-history.sqlite3`，重启后复用缓存；Codex 还保存文件身份和解析检查点，继续读取新增日志，未完整写入的行留待后续追加。

没有前端内存缓存时，先通过只读命令加载 `{codex,claude}-token-statistics.json`，无需等待日志扫描。每次展开或刷新仍在后台同步源日志变化并按当前本地日历聚合，随后将结果保存至统计 JSON 并读回更新前端缓存；这期间保留已有结果和真实统计时间。已有汇总不会阻止采集新增数据或重新计算日期范围。

会话只在 Rust 中解析，不把对话正文或原始日志发送到前端。模型金额采用核验的公开标准单价，无价格的模型保留 Token 统计；`estimatedCostUsd` 汇总可计价部分，并通过 `unpricedTokens` 提示不完整，全部无法计价时为 null。费率来源与边界记录在 [计价依据](token-pricing.md)。本地数据可能跨账号，不能与当前账号额度比例混为一谈。

## 菜单栏摘要

macOS 托盘标题格式为 `<剩余占比> <距离重置时长>`，例如 `60% 2天 20小时`。优先选启用且读取成功的 Codex 第一个有效额度窗口（核心窗口在前），不可用时选 Claude；剩余比例为 `100 - usedPercent`，限制在 0–100 后取整数。提示文字说明账号服务及具体窗口。没有有效用量时清空摘要，避免继续显示旧数值；重置时间未知或已到期时明确标识。

首次采集、手动/后台刷新和设置保存都会同步摘要；独立的 30 秒计时器仅从内存重算倒计时，不增加网络查询。托盘更新在主线程读取最新快照，防止旧事件将已关闭的服务重新显示。Tauri 的 Windows 托盘不支持标题，Linux 标题能否显示由桌面环境决定；当前验证目标为 macOS。

## 桌面与存储

主面板默认隐藏，左键托盘图标展开 360×600 逻辑像素面板；再次点击、失焦、Esc、收起按钮可隐藏。后台刷新独立于 WebView。macOS 使用 NSStatusItem 屏幕坐标定位；Linux 通过托盘菜单打开。Windows、Linux 的原生行为仍需独立验收。

统计数据统一保存在用户主目录的 `.agent-bar/`。`dashboard.json` 保存用于展示的账号元信息、套餐、额度和更新时间；两个服务各自的 SQLite 保存归一历史，统计 JSON 保存聚合结果。这里不保存账号凭据、额度接口原始响应或对话正文。macOS / Unix 目录权限为 `0700`，JSON 和 SQLite 文件权限为 `0600`。旧 Tauri 应用缓存保持原样，首次使用新目录时从源日志重建统计缓存。

设置继续保存在 Tauri 应用配置目录 `settings.json`；macOS 当前为 `~/Library/Application Support/dev.agentbar.desktop/settings.json`。浏览器设置保存在 `agentbar.settings.v1`，与桌面不互通。

开机启动、自动更新、应用签名和分发尚未接入。Codex 采集与历史统计参照 CodexBar，以 Rust 实现，未分发其 Swift 源码或 CLI；参照版本和 MIT 声明见 [第三方声明](third-party-notices.md)。AgentBar 的开源许可证尚未确定。
