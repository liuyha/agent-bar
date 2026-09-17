# AgentBar

使用 **Tauri 2 + React 19 + TypeScript + Vite + shadcn/ui + Tailwind CSS + Rust** 构建的 AI 工具用量托盘应用。读取本机已登录的 Codex、Claude Code 账号，在菜单栏查看真实用量。

没有演示模式。Codex 参照 CodexBar 的自动采集路径，读取本机 PAT / OAuth 查询账号额度，并在适用时回退到官方 CLI app-server；Claude 使用 Claude Code 本地 OAuth 登录态查询官方订阅用量。未登录或无法读取时显示对应状态，不填充示例数据。采集逻辑使用 Rust 实现，未分发 CodexBar 的 Swift 源码或 CLI。macOS 是首个开发与验证平台；Windows、Linux 保留 Tauri 工程骨架，兼容性仍需在对应系统验证。

## 当前功能

- 本机真实账号、套餐、用量比例与重置倒计时；支持多个用量窗口及单服务错误提示。
- 悬停或聚焦服务卡片，右侧展开今日、本周、本月、本年、全部的 **Token 用量、约等金额（USD）、请求数、会话轮次**。桌面窗口自动扩展，移出统计区域后收起；窄屏改为上下排列。
- macOS 菜单栏图标旁显示“剩余占比 距离重置时长”，如 `60% 2天 20小时`。优先显示 Codex 主额度，不可用时显示 Claude；用量更新时同步，倒计时每 30 秒重算。
- 启动后收起到菜单栏／系统托盘；左键点击图标，在图标旁直接展开统计面板，再次点击、点击外部、按 Esc 或点击收起按钮隐藏。
- 面板无标题栏，随托盘所在屏幕定位；右键菜单提供显示面板、偏好设置、刷新与退出入口；偏好设置在独立窗口打开，首页仅展示用量。
- 手动刷新与 Rust 后台定时刷新，窗口隐藏后仍继续刷新。
- 账号用量快照和历史统计保存在用户目录 `~/.agent-bar/`，重启先恢复已保存的用量，再后台更新真实数据。
- 设置服务启用状态、60／300／900 秒刷新间隔，以及跟随系统／浅色／深色主题。
- 桌面设置由 Rust 保存为本地 JSON；浏览器预览使用独立的 `localStorage`。

账号登录由各工具自行完成；AgentBar 读取现有登录态。开机启动、自动更新、应用签名和安装包分发尚未接入。Linux 的 Tauri 托盘不提供点击事件，需通过托盘菜单“显示面板”打开。当前检查及各平台实际验收范围见 [验证记录](docs/verification.md)。

## 开发环境

- 推荐 Node.js 24 LTS；使用 `package.json` 的 `packageManager` 所锁定的 pnpm 11.24.0，可用 `npm install --global pnpm@11.24.0` 安装。Node 24 的发布状态和 pnpm 11 兼容性分别见 [Node.js 发布表](https://nodejs.org/en/about/previous-releases)及 [pnpm 兼容表](https://pnpm.io/installation#compatibility)。
- 桌面开发需要 Rust stable，通过 [rustup](https://www.rust-lang.org/tools/install) 安装。`rust-toolchain.toml` 声明工具链及 `rustfmt`、`clippy` 组件。
- macOS 安装 Xcode Command Line Tools：`xcode-select --install`。
- Windows 需要 Microsoft C++ Build Tools 和 WebView2；Linux 需要 WebKitGTK 等系统库。具体依赖以 [Tauri 官方环境准备](https://v2.tauri.app/start/prerequisites/)为准。

仅运行浏览器预览时不需要 Rust 和桌面系统依赖。

## 界面组件

`src/components/ui/` 保存按 AgentBar 紧凑尺寸调整的 shadcn/ui 源码（Button、Checkbox、NativeSelect、Progress），`components.json` 配置组件路径，`@/` 指向 `src/`。组件来源为 [shadcn/ui 官方仓库](https://github.com/shadcn-ui/ui)，许可证保留在 [licenses/shadcn-ui-MIT.txt](licenses/shadcn-ui-MIT.txt)。设置、刷新操作、剩余额度和统计时段已使用这些组件或 Tailwind 工具类。窗口布局及复杂统计排版继续由现有 CSS 管理。

采用 **Tailwind CSS 3.4** 与 PostCSS，配合 `tailwind-merge` 2.x；保留现有样式重置，关闭 Preflight。主题颜色映射到已有 CSS 变量，因此浅色、深色和跟随系统共用同一套组件。现有构建目标包含 Safari 15 / macOS 12；[Tailwind CSS 4 需要 Safari 16.4 及以上](https://tailwindcss.com/docs/compatibility)，升级前需同步评估 WebView 兼容范围。统计来源使用原生下拉框，避免浮层脱离悬停区域后触发详情收起。

## 运行

在项目根目录执行：

```sh
pnpm install --frozen-lockfile
pnpm run doctor
```

浏览器预览：

```sh
pnpm dev
```

打开终端给出的本地地址；偏好设置预览地址为 `/#settings`。浏览器只能显示设置和“请使用桌面端”的账号状态，不会读取本机凭据或生成虚构用量。真实账号、托盘、后台刷新需要桌面运行。

启动桌面应用：

```sh
pnpm desktop:dev
```

Tauri 会启动 Vite 并编译 Rust。首次运行需要下载 Rust 依赖，耗时取决于网络与构建环境。桌面脚本会自动识别默认的 `~/.cargo/bin`，无需修改系统 `PATH`。运行桌面开发模式前，请先停止单独运行的 `pnpm dev`，避免占用同一个 1420 端口。

应用首次启动时不弹出窗口，请点击菜单栏／系统托盘中的 AgentBar 柱状图标查看统计。应用采用单实例模式，再次启动会展开已有面板。开发前请从托盘菜单退出已运行的打包版本，否则只会恢复已有面板。

## 本机账号读取

- **Codex**：读取 `$CODEX_HOME/auth.json`（默认 `~/.codex/auth.json`），依次使用可用的 PAT、OAuth 和受控 CLI 回退。OAuth 直接查询账号用量，PAT 先确认身份再查询；CLI 路径使用 `account/read` 和 `account/rateLimits/read`。保留服务端返回的实际窗口和模型专属额度。网络错误、限流不会触发额外 CLI 请求；API key 模式没有 ChatGPT 订阅限额。
- **Claude**：先通过 Claude Code 登录订阅账号。macOS 从 Keychain 的 Claude Code 登录项读取，其他平台读取本地凭据文件；API key 或代理模式不提供 Claude 订阅限额。
- 凭据留在 Rust／官方 CLI 内，前端只显示账号元信息和用量。AgentBar 不修改共享 `auth.json`；需要恢复原生登录时交给官方 CLI。未登录、凭据过期、服务错误都有明确提示。
- 关闭面板仍会后台刷新；修改启用服务后立即按新设置采集。

## 本地历史统计

Codex 使用统计提供 **本机记录／服务端** 两个来源，均可切换今日／本周／本月／本年／全部。程序自动选择服务端认证和查询方式；网页补充默认关闭。服务端按已返回每日记录汇总所选时段，“全部”采用服务端累计值，缺失日期不补零。网页补充独立展示 Credits、使用分布及代码审查额度。配置和数据边界见 [账号服务端使用统计](docs/account-statistics.md)。

统计读取 Codex 的 `sessions`、`archived_sessions` 和 Claude Code 的 `projects` 会话记录，尊重 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。Codex 还汇总 pi / OMP 会话中的 `openai-codex` 用量。无需账号额度接口成功也能查看本地历史，普通浏览器无法读取这些记录。

- 今日从本地时间零点开始；本周从周一零点开始；本月从一号零点开始；本年从 1 月 1 日零点开始；全部覆盖本机保留的所有历史记录。均统计至读取时刻，跨年日期显示年份。
- Token 数量按十进制自动显示 K／M／B 单位，悬停可查看精确值；请求数和会话轮次保持完整整数。
- Token 总量为输入（含缓存读取与写入）加输出。推理 Token 已包含在输出中，不重复累加。
- 请求数按可识别的模型调用去重；会话轮次按用户发起的交互轮次统计，不将一轮中的多个模型请求或工具调用算成多轮。缺少可识别记录的计数显示 `—`，真实零值显示 `0`。
- Codex、Claude 历史分别使用 `~/.agent-bar/` 中的 SQLite 保存标准化用量，支持跨启动复用；Codex 还保存解析检查点以增量读取追加日志。分叉继承记录、累计快照和请求明细分别处理，避免重复计算。每次加载或刷新仍同步源日志并按当前日历重新统计。
- 金额是按公开模型 Standard API 单价估算的美元等值，不代表订阅账单或实际扣费。部分 Token 缺少单价时仅显示已计价部分，并明确标注；全部无法计价时显示“暂无法估算”。计价表和适用边界见 [计价依据](docs/token-pricing.md)。
- 仅涵盖本机保留的日志，可能包含多个账号，无法代表其他设备或云端全部使用情况。缺失日志与读取错误不显示为零用量。

## 数据存储

桌面数据统一保存在用户目录 `~/.agent-bar/`：

- `dashboard.json`：账号用量展示快照，包含账号元信息、套餐、额度和更新时间。启动时按当前启用服务恢复；刷新写入并读回成功后才更新界面，存储失败保留原快照。
- `codex-token-history.sqlite3`、`claude-token-history.sqlite3`：标准化历史统计缓存。
- `codex-token-statistics.json`、`claude-token-statistics.json`：本次聚合结果，写入后读回供程序使用。
- `codex-account-statistics.json`：服务端成功统计缓存，按当前登录及配置范围核验后恢复；失败不覆盖成功数据。

本机统计再次展开时立即显示该服务上次缓存的结果，并在后台更新；只有没有缓存的首次加载显示“正在统计本机会话”。程序重新启动后先读取已保存的统计汇总，后台更新失败时保留已有结果并提示重试。

服务端统计启动先读缓存，有缓存不立即请求，无缓存才静默获取。主窗口按设置周期后台刷新，收起详情仍继续；自动刷新仅按钮旋转，手动刷新才显示面板 loading。刷新失败时保留仍匹配当前账号的成功缓存及原始采集时间。

数据不包含账号凭据、接口原始响应或对话正文。macOS / Unix 目录权限为 `0700`，JSON 和 SQLite 文件为 `0600`。旧应用缓存保留，新目录首次使用时从源日志重建。设置仍保存在 Tauri 应用配置目录的 `settings.json`，浏览器预览仍使用独立的 `localStorage`。

## 常用命令

| 命令 | 用途 |
| --- | --- |
| `pnpm dev` | 启动 Vite 浏览器预览 |
| `pnpm desktop:dev` | 启动 Tauri 桌面开发模式 |
| `pnpm typecheck` | TypeScript 类型检查 |
| `pnpm lint` | ESLint 检查 |
| `pnpm test` | 前端自动化测试 |
| `pnpm build` | 构建前端静态资源 |
| `pnpm check` | 执行前端类型、Lint、测试和构建检查 |
| `pnpm rust:check` | 执行 Rust 格式检查、Clippy 和测试 |
| `pnpm desktop:build` | 在当前系统构建桌面应用 |
| `pnpm run doctor` | 检查本机 Tauri 开发环境（`run` 避免调用 pnpm 内置的同名命令） |

提交依赖变更时一并维护 `pnpm-lock.yaml` 与 `src-tauri/Cargo.lock`。桌面构建产物不代表已经完成签名、跨平台验证或发布。

## 工程结构

```text
src/
  components/       用量面板与设置等 React 组件
  lib/api.ts        Tauri 调用与浏览器不可用状态
  lib/format.ts     展示格式化
  types/index.ts    前端数据契约
src-tauri/
  src/lib.rs        Rust 命令、托盘、窗口与后台刷新
  src/panel.rs      托盘面板定位、点击切换与失焦收起
  src/models.rs     Rust 数据契约和设置校验
  src/state.rs      应用状态与设置持久化
  src/storage.rs    用户数据目录、私有文件权限与原子读写
  src/providers.rs  Provider 接口与并行采集
  src/providers/   Codex / Claude 本机账号适配
  capabilities/     WebView 权限声明
  tauri.conf.json   桌面窗口与构建配置
docs/
  architecture.md   模块边界、通信契约与迭代路线
```

扩展 Provider 前请先阅读 [架构说明](docs/architecture.md)。桌面采集逻辑放在 Rust 中，React 只依赖统一的用量模型。

## 调研与许可

托盘能力基于 [Tauri 系统托盘 API](https://v2.tauri.app/learn/system-tray/)。后续可按需求评估独立 Rust 采集或 [sidecar](https://v2.tauri.app/develop/sidecar/)；当前未选择或实现 sidecar 集成。

Codex 额度与历史统计参照 [CodexBar](https://github.com/steipete/CodexBar) 实现，参考版本、对齐范围和验证见 [Codex 对齐说明](docs/codex-alignment.md)，上游 MIT 声明保留于 [第三方声明](docs/third-party-notices.md)。AgentBar 的开源许可证尚未确定。
