<p align="center">
  <img src="public/agentbar.svg" width="80" height="80" alt="AgentBar 标志" />
</p>

<h1 align="center">AgentBar</h1>

<p align="center"><strong>AI 用量，一目了然。</strong></p>
<p align="center">在菜单栏查看 Codex 与 Claude Code 的剩余额度、重置时间和 Token 使用统计。</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#数据与隐私">数据与隐私</a> ·
  <a href="#开发与贡献">开发与贡献</a> ·
  <a href="LICENSE">MIT License</a>
</p>

AgentBar 是一款基于 **Tauri 2、React、TypeScript 和 Rust** 的开源桌面应用。它读取本机已有的工具登录态，将账号额度与使用历史集中到菜单栏，方便你在工作时随时查看。目前以 **macOS** 为主要开发和验证平台。

## 核心功能

- **额度随时可见**：展示账号、套餐、各用量窗口的剩余比例和重置倒计时；macOS 菜单栏直接显示额度摘要。
- **按时段查看统计**：今日、本周、本月、本年、全部；本机记录汇总 Token、约等金额、请求数和会话轮次。
- **两种统计来源**：Codex 支持本机记录与账号服务端统计；Claude Code 支持本机记录。
- **独立统计窗口**：悬停服务卡片即可展开详情，支持键盘操作；偏好设置从托盘右键菜单打开。
- **后台刷新与本地缓存**：支持手动刷新和 1／5／15 分钟定时刷新，重启后恢复缓存，并标明数据更新时间。
- **跟随你的桌面**：支持浅色、深色和跟随系统主题，可单独启用或关闭服务。

## 支持范围

| 工具 | 账号额度 | 本机 Token 统计 | 服务端 Token 统计 |
| --- | --- | --- | --- |
| Codex | 本机订阅登录态；自动选择可用认证方式 | 会话与归档日志，也包含 pi / OMP 的 `openai-codex` 记录 | 当前账号返回的活动汇总与每日记录 |
| Claude Code | 本机 OAuth 订阅登录态 | 本机 `projects` 会话日志 | 暂不支持 |

账号登录和切换由原工具完成，AgentBar 读取现有登录态。API key 登录不提供订阅额度；本机历史统计独立于额度接口，仍取决于可读取的日志。

| 平台 | 当前状态 |
| --- | --- |
| macOS | 主要开发与验证平台，构建最低版本为 macOS 12 |
| Windows / Linux | 保留 Tauri 工程配置，尚未完成平台兼容性验证 |

当前提供源码构建流程，应用签名、自动更新和开机启动尚未接入。部分用量接口属于上游内部接口，兼容性可能随上游版本变化。

## 快速开始

### 1. 准备环境

| 依赖 | 要求 |
| --- | --- |
| Node.js | 24 或更高版本 |
| pnpm | 11.24.0，与 `package.json` 中的 `packageManager` 保持一致 |
| Rust | 通过 [rustup](https://www.rust-lang.org/tools/install) 安装 stable，工具链配置见 `rust-toolchain.toml` |
| 系统依赖 | macOS 需要 Xcode Command Line Tools；其他平台参考 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/) |

macOS 安装命令行工具：

```sh
xcode-select --install
```

请先在需要查看的 Codex 或 Claude Code 中完成登录。

### 2. 启动应用

```sh
git clone https://github.com/liuyha/agent-bar.git
cd agent-bar
npm install --global pnpm@11.24.0
pnpm install --frozen-lockfile
pnpm run doctor
pnpm desktop:dev
```

首次启动需要编译 Rust 依赖。应用启动后驻留菜单栏，点击 AgentBar 柱状图标打开主面板；右键菜单可进入偏好设置、刷新用量或退出。Linux 需从托盘菜单选择“显示面板”。

应用采用单实例模式。开发前请先从托盘退出已经运行的打包版本；如果单独启动了 `pnpm dev`，也请先停止它，避免占用桌面开发所需的 `1420` 端口。

### 3. 构建桌面应用

```sh
pnpm desktop:build
```

产物位于 `src-tauri/target/release/bundle/`，安装包格式随构建平台而定。该命令构建本机平台的应用，不代表已完成签名或其他平台验证。

仅预览前端时可运行 `pnpm dev`，打开 `http://127.0.0.1:1420`；偏好设置地址为 `/#settings`。浏览器预览不需要 Rust，也无法读取本机账号或会话日志，会显示桌面端使用提示。

## 数据与隐私

凭据读取、用量查询和日志解析在 Rust 或官方 CLI 中完成，前端只接收账号元信息与归一化统计。AgentBar 不要求你在界面中填写 API key，不自行改写共享登录文件；额度和服务端统计查询需要访问对应服务的接口。

| 数据 | 默认位置与用途 |
| --- | --- |
| Codex 登录态与日志 | `~/.codex/`，支持 `CODEX_HOME` |
| Claude Code 登录态与日志 | macOS 优先读取 Keychain，回退本地凭据文件；其他平台读取本地凭据。日志默认在 `~/.claude/projects/`，支持 `CLAUDE_CONFIG_DIR` |
| AgentBar 统计缓存 | `~/.agent-bar/`，保存账号用量快照、SQLite 历史缓存和统计汇总 |
| 偏好设置 | Tauri 应用配置目录中的 `settings.json`；macOS 为 `~/Library/Application Support/dev.agentbar.desktop/settings.json` |

统计缓存不保存原始凭据、接口原始响应或对话正文，但包含账号元信息和用量数据。macOS / Unix 上，数据目录权限为 `0700`，JSON 与 SQLite 文件权限为 `0600`。

理解统计结果时，请留意以下口径：

- **本机记录**仅覆盖本机保留的日志，可能包含多个账号；本周从本地时间周一开始，缺失数据与真实零值分别展示。
- **服务端统计**以当前账号实际返回的数据为准，可能存在延迟或日期缺失，与本机统计分开展示，不相加、不补零。
- **约等金额**按内置 Standard API 美元价格估算，并以固定汇率 **1 USD ≈ 7 CNY** 展示人民币。它不代表订阅账单或实际扣费；缺少模型价格时会标明无法估算或仅部分计价。

## 开发与贡献

欢迎通过 [Issues](https://github.com/liuyha/agent-bar/issues) 反馈问题，或提交 [Pull Request](https://github.com/liuyha/agent-bar/pulls)。问题报告请附操作系统、应用版本、复现步骤和脱敏后的错误信息；涉及界面修改时可附截图。

| 命令 | 用途 |
| --- | --- |
| `pnpm run doctor` | 检查本机开发环境；使用 `run` 避免调用 pnpm 内置同名命令 |
| `pnpm dev` | 浏览器预览 |
| `pnpm desktop:dev` | 桌面开发 |
| `pnpm check` | 前端类型检查、ESLint、测试和生产构建 |
| `pnpm rust:check` | Rust 格式检查、Clippy 和测试 |
| `pnpm desktop:build` | 构建当前平台桌面应用 |

提交前运行与改动相关的检查；涉及原生窗口、托盘或账号读取时，还需在桌面端验证。依赖变更应同步维护对应的 `pnpm-lock.yaml` 或 `src-tauri/Cargo.lock`。CI 配置见 [检查工作流](.github/workflows/ci.yml)。

```text
src/                 React 界面、状态管理与 Tauri 通信
src-tauri/src/       Rust 账号适配、统计、存储与原生窗口
src-tauri/icons/     桌面应用与托盘图标
scripts/            开发环境检查与桌面命令入口
docs/               架构、统计口径与第三方声明
licenses/           第三方许可证
```

进一步了解实现与数据边界：

- [架构说明](docs/architecture.md)：模块职责、数据流和扩展约定。
- [账号服务端统计](docs/account-statistics.md)：来源、刷新机制和指标含义。
- [Token 计价依据](docs/token-pricing.md)：内置价格表、核验日期和估算边界。
- [第三方声明](docs/third-party-notices.md)：参考项目、组件许可和图标来源。

## 许可证与致谢

AgentBar 采用 [MIT License](LICENSE)。第三方代码、资源和品牌标志遵循各自声明，详见 [第三方声明](docs/third-party-notices.md)。

感谢 [CodexBar](https://github.com/steipete/CodexBar) 提供账号采集与历史统计的实现参考，以及 Tauri、React、shadcn/ui 等开源项目。AgentBar 是独立项目，与 OpenAI、Anthropic 无隶属或背书关系。
