# 架构与数据契约

AgentBar 使用 Tauri 连接 React 界面与 Rust 采集层。React 负责展示和交互，Rust 负责本机凭据、网络请求、会话解析及持久化。普通浏览器只提供界面预览和独立设置，不读取本机账号。

## 模块边界

| 模块 | 职责 |
| --- | --- |
| `src/lib/api.ts`、`src/types/index.ts` | Tauri 调用、事件订阅与前端数据契约 |
| `src-tauri/src/models.rs` | Rust 数据契约、设置默认值与校验 |
| `src-tauri/src/providers.rs`、`providers/` | Provider 接口、并行采集和 Codex / Claude 适配 |
| `src-tauri/src/state.rs`、`storage.rs` | 设置、账号快照、并发控制与私有文件存储 |
| `src-tauri/src/statistics.rs`、`statistics/` | 本机会话解析、去重、增量缓存、时间聚合与计价 |
| `src-tauri/src/account_statistics.rs`、`account_statistics_cache.rs`、`providers/codex/activity.rs` | Codex 服务端统计及账号范围隔离 |
| `src-tauri/src/lib.rs`、`tray_summary.rs` | 命令注册、后台采集、托盘菜单与摘要 |
| `src-tauri/src/panel.rs` | 原生窗口定位、高度调整、焦点与跨窗交互 |
| `src/components/StatisticsWindow.tsx`、`TokenStatisticsPanel.tsx` | 统计窗口与本机 / 服务端统计展示 |
| `src/lib/tokenStatistics.ts`、`accountStatistics.ts`、`useAccountStatistics.ts` | 统计缓存、请求合并和刷新生命周期 |
| `src/lib/panel.ts`、`useContentWindowHeight.ts` | 窗口操作队列、版本确认与内容高度测量 |

新增 Provider 应在 Rust 中实现采集，沿用统一模型，不向前端暴露凭据或引入本地 HTTP 凭据服务。

## 数据与通信

Rust 与 TypeScript 使用 camelCase JSON 契约。字段变更需同步两端模型及消费者。

- `ProviderUsage` 包含服务、账号、套餐、状态、额度窗口和采集时间。`status` 为 `ready`、`unavailable` 或 `error`；未登录和不支持的认证模式为不可用。窗口只包含实际返回的 `label`、`usedPercent` 和可空的 `resetsAt`，不伪造缺失额度。
- Codex 套餐依据实际 `plan_type` / `planType` 展示：`prolite` 为 `Pro 5x`，`pro` 为 `Pro 20x`。`resetCredits` 包含可空总数 `remaining`、可空明细 `credits`、独立采集时间 `updatedAt` 和提示 `message`；每项明细包含 `id`、`remaining`、可空的 `expiresAt`。旧快照缺少该字段时视为未知；`null` 与确认返回的零次 / 空列表区分。
- `DashboardSnapshot` 包含 `providers`、采集尝试时间 `updatedAt`、`mode: live` 和单调递增的 `revision`。前端拒绝较低版本，防止事件与命令返回乱序覆盖。
- `AppSettings` 默认启用两个服务、300 秒刷新、跟随系统主题和本机统计。支持 60 / 300 / 900 秒刷新、system / light / dark 主题；`codexStatisticsSource` 为 local / auto，旧版显式认证来源迁移为 auto。
- `TokenStatistics` 与服务端 `AccountUsageSnapshot` 是独立契约。可空计数、价格和日期表示未知，与真实零值区分；两类统计不相加、不互相补零。

| 命令组 | 契约 |
| --- | --- |
| `get_dashboard`、`refresh_dashboard`、`refresh_provider_dashboard` | 读取快照、刷新全部启用服务或指定服务 |
| `get_settings`、`save_settings` | 读取、校验并保存设置，应用服务过滤并触发后台采集 |
| `get_cached_token_statistics`、`get_token_statistics` | 仅读已存汇总，或同步指定服务日志并重新聚合 |
| `get_cached_codex_account_statistics`、`get_codex_account_statistics` | 仅读匹配当前登录范围的缓存，或采集服务端统计 |
| `show_statistics_panel`、`get_statistics_panel_state`、`present_statistics_panel` | 请求展开、读取选择状态、确认对应版本已渲染 |
| `resize_content_window`、`set_panel_interaction` | 上报内容高度与交互意图，由原生层调整窗口 |
| `hide_panel`、`hide_settings`、`hide_statistics_panel`、`dismiss_panel` | 隐藏指定窗口或按层级关闭详情及主面板 |

`usage-updated`、`settings-updated` 同步数据；`statistics-panel-changed` 同步 `{ provider, side, revision }`。统计窗口先订阅再读取状态，完成高度同步和渲染后确认版本；原生层拒绝已关闭或切换服务后的迟到确认。`refresh-token-statistics` 携带目标服务，`refresh-account-statistics` 将手动操作交给唯一的服务端统计 store。

## 认证与刷新

Codex 读取 `$CODEX_HOME/auth.json`，默认目录为 `~/.codex`。采集依次使用可用 PAT、OAuth 和受控的官方 CLI 回退。PAT 先确认身份，OAuth 使用当前凭据的账号 ID；显式配置目录不借用其他目录的身份。网络错误、服务端错误和限流不触发额外 CLI 请求。自定义 backend 交给同一配置范围的 CLI 处理，HTTP 路径不向任意配置地址转发令牌。额度 CLI 调用使用 `account/read` 与 `account/rateLimits/read`，不启动模型任务，也不由 AgentBar 写回共享凭据文件。

Codex 重置总数使用 `/wham/usage` 的 `rate_limit_reset_credits.available_count`；`applicable_available_count` 表示当前可兑换次数，不作为剩余数量展示。HTTP 另外只读请求 `/wham/rate-limit-reset-credits` 获取状态和到期时间，CLI 读取 `rateLimitResetCredits`。重置明细失败不影响已获取的额度窗口及已知总数，明细可能截断，不能用列表长度覆盖服务端总数。卡片点击“重置剩余”展开列表，按到期时间升序、以本机时区展示完整日期和时分；已知明细到期后从显示总数扣除，未知到期时间保留为未知。此入口只查看，不兑换或消费重置次数。

Claude 使用 Claude Code 已有的订阅 OAuth 登录态。macOS 优先读 Keychain，再回退到本地凭据文件；其他平台使用本地文件，尊重 `CLAUDE_CONFIG_DIR`。API key 和代理模式不提供订阅额度。凭据只在 Rust / 官方 CLI 内处理，前端接收账号元信息、归一用量和脱敏错误。

账号额度刷新在 Rust 后台执行，不依赖窗口是否显示。两个服务并行采集，错误分别展示；刷新互斥防止重复采集，凭据和网络操作不持有设置锁。设置变更使在途旧采集失效。有效快照写入并读回成功后才发布；存储失败保留原内存快照。

连接失败时，仅在登录范围及账号匹配的情况下保留已有额度和原采集时间，并继续标识错误。未知范围、账号变化或认证不可用不会套用其他账号的数据。用于比较的范围摘要保存在 Rust 和私有磁盘结构中，不传入 WebView。

服务端统计的初始化、定时和手动刷新由常驻 `StatisticsWindow` 中的 `useAccountStatistics` 唯一持有，主窗口与设置窗口不重复轮询。有效缓存先展示，无缓存再静默采集；后台更新保留内容，手动刷新显示加载反馈，在途请求共用。账号或来源变化取消旧生命周期，Rust 在采集及缓存读写前后复核登录范围。详细字段与刷新规则见 [服务端统计](account-statistics.md)。

## 本地统计与计价

Codex 读取 `sessions`、`archived_sessions`，另读取 `~/.pi/agent/sessions`、`~/.omp/agent/sessions` 中 provider 为 `openai-codex` 的记录；Claude 读取 `projects`。源目录尊重 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。统计仅覆盖本机保留日志，可能跨账号，不能视作当前账号的全部用量。

解析和聚合遵循以下约束：

- 模型来自日志的明确标记；`token_usage_record` 单次请求与 `token_count` 累计快照分别处理，不直接相加。重复文件、响应以及 pi / OMP 的同 session、同 entry 记录去重。
- 分叉继承记录与继承累计基线不计为新用量；子智能体以 `subagent_history_start_ordinal` 等日志标记识别自有历史边界。
- Codex SQLite 缓存保存标准化记录、文件身份和解析检查点。未变文件复用结果，追加日志从已提交字节位置继续解析；不完整 JSON 留待后续追加。文件替换、截断、重写或检查点不匹配时重新解析。
- 请求数统计可识别模型调用，会话轮次统计用户发起的交互；一轮内多次请求或工具调用不等于多轮。输入总量包含缓存读取与写入，推理 Token 已在输出中，不重复累加。
- 今日、本周、本月、本年按本机日历零点划分，周一为每周起点；全部覆盖可读取的历史，截至读取时刻。读取失败、缺失日志和无法可靠重建的计数保留缺失或不完整状态。
- `TokenStatistics.activity` 保存全部本机历史的最长已完成主任务时长、当前连续活跃和最长连续活跃；前端与服务端复用三列概览。活跃日按本地日历去重，当前连续可以延续到昨天，缺少任务时长证据保持 null。旧汇总缺少字段仍可读取，解析器版本升级会重新扫描历史补齐活动数据。具体口径见 [本机活动概览](account-statistics.md#本机活动概览)。

本机统计先读取内存或已保存的汇总 JSON，再后台同步源日志。关闭详情不丢弃进行中的请求，同服务并发刷新合并；刷新失败保留已有结果及原统计时间。

Rust 按精确模型映射计算 Standard API 美元等值，保存 `estimatedCostUsd` 和 `unpricedTokens`。未知模型不套用同族价格，全部不可计价时金额为 null。前端按固定估算汇率 `1 USD ≈ 7 CNY` 换算一次；该金额不代表订阅账单或实际扣费。费率、长上下文与缓存计价边界见 [计价依据](token-pricing.md)。

## 原生窗口与托盘

应用默认收起到托盘，采用单实例模式。主面板宽 320、统计窗口宽 360 逻辑像素，初始高度 600；实际高度随内容测量调整并限制在屏幕工作区，不能将初始尺寸视为固定窗口尺寸。偏好设置为独立窗口，关闭时隐藏。

统计窗口启动时预创建并复用，优先位于主面板右侧，空间不足时翻转到左侧；狭窄工作区会缩窄或在工作区内覆盖展示。跨窗口操作串行提交，高度和渲染确认携带选择版本，避免迟到回调改变新窗口状态。

原生层采样真实鼠标坐标，以当前服务卡片、可见统计窗口和窄过渡间隙为保留区域；连续离开约 250 ms 后收起，关闭前复核位置和版本。键盘意图可以保留面板，实际鼠标移动后恢复位置判断；旧 DOM 焦点不阻止收起。悬停展开不抢焦点，点击或方向键进入详情，Esc 逐级关闭，点击两窗之外隐藏整组。

额度采集独立于 WebView。统计 WebView 请求禁用后台节流，macOS 14 及以上支持此能力；其他系统版本的隐藏执行时机受 WebView 限制。原生窗口行为以 macOS 为主要验证目标，Windows / Linux 仍需对应平台验收，Linux 通过托盘菜单打开面板。

macOS 托盘摘要显示剩余比例和重置倒计时，优先选读取成功的 Codex 主要额度窗口，否则选择 Claude；无有效用量时清空。30 秒计时器只从已有快照重算倒计时，不增加网络请求。

## 存储

| 路径 | 内容 |
| --- | --- |
| `~/.agent-bar/dashboard.json` | 账号元信息、套餐、额度、采集时间与私有范围摘要 |
| `~/.agent-bar/{codex,claude}-token-history.sqlite3` | 标准化历史记录；Codex 另含增量解析检查点 |
| `~/.agent-bar/{codex,claude}-token-statistics.json` | 本机时段聚合结果 |
| `~/.agent-bar/codex-account-statistics.json` | 当前账号和配置范围匹配的服务端成功缓存 |
| Tauri 应用配置目录的 `settings.json` | 桌面偏好设置 |

统计缓存不保存认证令牌、对话正文或接口原始响应。macOS / Unix 数据目录权限为 `0700`，JSON 与 SQLite 文件权限为 `0600`，JSON 采用原子写入。旧 Tauri 缓存保持原样，新目录从源日志重建。

macOS 设置路径为 `~/Library/Application Support/dev.agentbar.desktop/settings.json`；浏览器预览使用 `agentbar.settings.v1` 的 localStorage，与桌面设置不互通。GitHub Actions 按版本标签构建各平台安装包并汇总到 Release 草稿，操作见 [发布指南](releasing.md)。macOS 构建使用 ad-hoc 签名，开发者证书、公证、Windows 代码签名、开机启动与自动更新尚未接入。上游参考与第三方许可见 [第三方声明](third-party-notices.md)，项目许可见 [MIT License](../LICENSE)。
