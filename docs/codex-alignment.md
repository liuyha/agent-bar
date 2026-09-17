# Codex 额度与 Token 历史对齐

参考 [CodexBar 提交 `2d9334237e7c48cf18799c7302ce3229069f3a81`](https://github.com/steipete/CodexBar/tree/2d9334237e7c48cf18799c7302ce3229069f3a81)，对齐日期为 2026-09-17。以该提交的源码为准；其文档中的部分自动选择顺序落后于实现。AgentBar 以 Rust 实现采集、解析与缓存，保留现有 Tauri / React 界面及数据契约。

## 账号额度

- 本机 `CODEX_HOME/auth.json` 中的 PAT 优先；存在可用 OAuth 时请求 `https://chatgpt.com/backend-api/wham/usage`。
- PAT 通过 OpenAI 的 `whoami` 确认账号，再以该账号请求额度；OAuth 使用当前凭据的账号 ID。显式 `CODEX_HOME` 不借用默认目录的身份。
- 无法直接取得可用原生认证时，交给同一配置范围的 `codex app-server`，使用 `account/read` 与 `account/rateLimits/read`。AgentBar 不自行刷新或写回 `auth.json`。
- 按失败类型处理回退，网络失败、服务端错误、限流不会一律触发 CLI。自定义 backend 由官方 CLI 处理，AgentBar 的 HTTP 路径不向任意配置地址转发令牌。
- 解析服务端实际额度窗口、重置时间和模型附加额度；不存在的百分比、重置时间或窗口不补零。API key 模式保持订阅额度不可用。
- 凭据及原始响应只在 Rust 中处理，错误对前端使用固定说明。

上游参考：[选择策略](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Sources/CodexBarCore/Providers/Codex/CodexProviderDescriptor.swift)、[OAuth](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Sources/CodexBarCore/Providers/Codex/CodexOAuth/CodexOAuthUsageFetcher.swift)、[PAT](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Sources/CodexBarCore/Providers/Codex/CodexPAT/CodexPATUsageFetcher.swift)。RPC 依据 [OpenAI 官方 app-server 文档](https://learn.chatgpt.com/docs/app-server)。

## 本地历史

原生来源为 `$CODEX_HOME/sessions` 与 `$CODEX_HOME/archived_sessions`，未指定时使用 `~/.codex`。额外读取 `~/.pi/agent/sessions`、`~/.omp/agent/sessions` 中 provider 为 `openai-codex` 的 assistant usage，不把其他服务商的记录计入 Codex。

模型使用 `turn_context` 等明确标记；原生 `token_usage_record` 单次请求与 `token_count` 累计快照分别解析，避免把两者相加。重复文件、同一响应和 pi / OMP 的同 session、同 entry 记录去重。子智能体的 `subagent_history_start_ordinal` 是其自有历史边界；父会话继承记录和继承累计基线不作为新的用量。

Codex 历史缓存使用应用缓存目录内的 `codex-token-history.sqlite3`，保存标准化用量、文件身份和解析检查点。未改变的文件复用结果，追加日志从已提交的字节位置继续解析；半行保留到后续追加完成。文件替换、截断和检查点不匹配会重新解析。缓存不保存对话正文或认证令牌，缓存失败与不完整日志有明确状态。

上游参考：[成本扫描器](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Sources/CodexBarCore/Vendored/CostUsage/CostUsageScanner.swift)、[pi 来源](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Sources/CodexBarCore/PiSessionCostScanner.swift)、[分叉边界回归](https://github.com/steipete/CodexBar/blob/2d9334237e7c48cf18799c7302ce3229069f3a81/Tests/CodexBarTests/CodexSubagentOrdinalBoundaryTests.swift)。

## AgentBar 保留的产品边界

- 今日、本周、本月、本年、全部继续采用本机日历；与 CodexBar 的滚动历史窗口需选择相同日期后才能对账。
- 继续展示可识别请求数和用户会话轮次，保留未知计数和未知价格；Claude 的采集与统计规则不随本次 Codex 对齐改变。
- 金额仍按 [现有 Standard API 计价表](token-pricing.md)计算；此任务没有迁移 CodexBar 的在线价格目录、Priority 跟踪或账单功能。
- 额度采用默认账号采集链路；网页 Cookie 附加信息、Credits 余额、账号切换和工作区管理需要各自的界面与账号状态设计，不属于现有额度卡片。
- 两个项目没有共享缓存数据库，也不互相调用 CLI。相同来源不代表不同时间、范围或价格表下的结果逐字相同。

测试与本机验收记录见 [验证记录](verification.md)。
