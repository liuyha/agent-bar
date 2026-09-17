# 本机验证记录

下面的早期阶段记录保留为历史；真实账号接入后的验证见文末。

2026-09-16，在 Apple Silicon macOS 上完成。工具链为 Node.js 26.8.1、pnpm 11.24.0、Rust 1.98.1，桌面使用系统 WebKit。

## 自动检查

- `pnpm install --frozen-lockfile`：依赖锁文件可用。
- `pnpm run doctor`：Node、pnpm、Rust、Cargo、Xcode Command Line Tools 均可用。
- `pnpm check`：TypeScript、ESLint、17 个测试、Vite 生产构建通过。
- `pnpm rust:check`：格式、Clippy 严格检查、8 个 Rust 测试通过。
- `pnpm desktop:build --debug --bundles app`：生成本地调试版 `src-tauri/target/debug/bundle/macos/AgentBar.app`。
- `pnpm desktop:dev`：Vite 和 Rust 开发命令成功启动；已有实例时恢复原窗口并退出第二个进程。

## 交互检查

浏览器验证了两个服务的演示用量、刷新时间更新、主题保存、服务全关空状态、重新启用，以及刷新页面后保留设置。检查期间无浏览器 warning/error 日志。

原生应用从 `tauri://localhost` 成功加载界面并调用 Rust。保存“一分钟刷新、仅 Claude、深色”后，界面及 macOS 配置文件内容一致。关闭窗口后进程继续存活；恢复窗口后，更新时间从 18:52:57 自动推进到 18:53:57，验证隐藏期间后台刷新仍运行。测试结束恢复“两项服务、五分钟刷新、跟随系统”。

## 菜单栏统计面板改动（同日）

- `pnpm check`：TypeScript、ESLint、17 个前端测试和 Vite 构建通过。
- `pnpm rust:check`：Rustfmt、Clippy 严格检查及 13 个 Rust 测试通过。新增 5 个定位测试覆盖菜单栏下方、屏幕右边缘、底部／侧边任务栏、Retina 与负坐标屏幕、小尺寸可用区域。
- `pnpm desktop:build --debug --bundles app`：重新生成 `src-tauri/target/debug/bundle/macos/AgentBar.app`。
- 已启动重新打包的程序，观察到新进程持续运行，启动输出无错误。启动成功不等于点击行为通过。
- 已做独立代码复核；macOS 定位使用实际 `NSStatusItem` 的屏幕和逻辑坐标，避免不同缩放比例下物理屏幕坐标相互重叠。

本次原生验收被锁屏阻断：Computer Use 首次返回 Mac 已锁定且无法自动解锁，后续检查超时。因此尚未实测本次面板的图标展开／再次点击收起、点击外部／Esc 收起、设置下拉框、不同显示器切换及隐藏后后台刷新。上方“交互检查”是改动前版本的记录，不能替代本次验收。

待解锁后按以下顺序验收：首次启动无普通窗口 → 左键图标直接显示统计面板且位置贴近图标 → 再次左键收起 → 重开后点击外部收起 → 重开后按 Esc 收起 → 右键“设置…”能打开设置且下拉框可正常选择 → 收起后重开返回统计页 → 验证背景刷新及设置仍保留。

## 验证范围

托盘左右键和新面板尚未完成原生点击验收。Windows、Linux、发行构建及签名、公证、真实账号用量均未验证；Linux 受 Tauri 托盘事件限制，通过菜单“显示面板”打开。该阶段交付使用演示数据；后续真实账号接入已将其移除。

## 真实账号接入（2026-09-16）

- 已移除 Rust 与浏览器演示 Provider，React 不再显示演示横幅、固定套餐或固定百分比。
- `pnpm check`：TypeScript、ESLint、26 项前端测试与生产构建通过。覆盖浏览器不伪造用量、桌面 IPC、账号状态、缺失重置时间和多额度窗口。
- `pnpm rust:check`：Rustfmt、Clippy（`-D warnings`）、24 项离线测试通过；2 项真实读取测试默认 ignored，另行执行。覆盖解析、API Key/未登录、子进程超时清理、设置保存与采集并发。
- `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib -- --ignored --nocapture`：2 项本机真实读取通过。Codex 读取到 Pro 账号与三个真实窗口，检查当时核心每周使用 39%，Spark 五小时/每周使用 0%；Claude 返回未检测到订阅登录，无账号及用量。测试输出仅使用掩码邮箱，没有输出凭据。
- 浏览器直接访问和手动刷新均显示“请通过桌面应用运行 AgentBar，读取本机已登录的账号”，没有百分比或示例套餐。已目视检查布局。
- `pnpm desktop:build --debug --bundles app`：打包成功并启动新版。原生 `tauri://localhost` 面板实际显示已登录 Codex 邮箱、Pro、39% 核心每周用量及两个 0% Spark 窗口；目视确认三窗口布局正常。刷新后检查时间从 19:35:46 推进到 19:36:19。保留用户当前的服务启用设置。
- 订阅 `usage-updated` 成功后重新读取缓存，补齐首次读取与事件订阅之间的空窗；revision 合并避免覆盖更新数据。

Claude 已登录后的成功 HTTP 路径尚不能在此机器验收，因为本机没有 Claude 登录态。Windows/Linux 原生采集与签名发行构建仍未验收。

## 菜单栏摘要（2026-09-16）

- macOS 图标旁新增 `剩余占比 距离重置时长`，如 `60% 2天 20小时`。主额度优先，提示文字注明来源服务和窗口；没有可用数据时清空旧摘要。
- 用量采集与设置保存后立即更新；独立 30 秒计时器重算倒计时，面板隐藏后继续运行，不增加账号请求。
- 新增 8 项摘要测试，覆盖剩余比例换算、取整、分钟/小时/天边界、时区、过期、未知重置、服务回退与无效用量。`pnpm rust:check` 串行执行通过（32 项离线测试，2 项真实账号测试保持忽略），桌面调试应用重新打包成功。
- 本次原生 UI 工具对 AgentBar、系统菜单栏和截图操作均返回超时，无法完成菜单栏文本和点击展开位置的目视验收。已运行的旧实例需要重启以加载新代码；上节原生面板记录不能替代本次摘要验收。

## 悬停历史统计（2026-09-16）

- 服务卡片悬停、聚焦或点击时展示右侧详情，按今日／本周／本月统计 Token、约等金额（USD）、请求数和会话轮次。按本地时区划分日历周期，本周从周一开始。
- `pnpm check` 通过：TypeScript、ESLint、36 项前端测试和 Vite 构建。覆盖三周期四指标、未知与零区分、部分计价、IPC 参数和扩展命令顺序。
- `pnpm rust:check` 通过：Rustfmt、Clippy（`-D warnings`）、54 项离线测试；3 项本机读取测试默认 ignored。新增统计覆盖日历跨年／本地边界、现代请求与旧累计记录去重（包括延迟快照）、分支复制／归档去重、用户轮次与子智能体区分、Claude 分块／工具返回、文件缓存更新、缺失基础 Token 字段、未知模型及部分计价。
- 单独执行 `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib statistics::tests::local_statistics_smoke -- --ignored --nocapture` 通过。真实 Codex 日周月四项统计均可读取，首次扫描约 8.54 秒，缓存重读约 30.5 毫秒；本机 Claude 没有会话记录，返回 unavailable，未伪造为零用量。输出仅含聚合统计及耗时，不输出对话或凭据。
- 浏览器实际验证纯鼠标悬停展开、指针移入右侧详情保持、切换服务和移出收起。普通浏览器仍明确提示通过桌面端读取日志，无演示用量。
- 使用临时隔离测试页检查三周期四指标排版、大数值、700px 展开布局及 360px 单列适配，无横向溢出；临时测试页已删除。Token 明细默认折叠，主要四指标保持可见，内容超出时可滚动。
- 新增 4 项原生扩展定位测试；屏幕右侧不足时左移并限制在工作区内，收起恢复原位置。展开先扩尺寸再移动，前端延迟确认鼠标移出，避免中间几何变化引发反复开合。
- `pnpm desktop:build --debug --bundles app` 通过，已生成包含本次功能的 `src-tauri/target/debug/bundle/macos/AgentBar.app`。

原生 UI 自动化本次获取应用返回 `timeoutReached`，因此浏览器交互检查和定位单元测试不能替代 macOS 真正的托盘悬停／展开／收起验收。Claude 的真实日志成功路径使用离线样例覆盖；本机尚无对应日志可实测。Windows／Linux、多显示器热插拔及签名发行仍未验收。


## 本年与全部统计（2026-09-17）

- 统计时段顺序为今日／本周／本月／本年／全部。新增范围覆盖 Token、约等金额、请求数和会话轮次；本年从本地元旦零点开始，全部从最早有效本机记录开始，均排除读取时刻之后的记录。
- `pnpm check` 通过：TypeScript、ESLint、75 项前端测试与 Vite 生产构建；验证五周期切换、缺失范围和跨年日期显示。
- `pnpm rust:check` 复跑通过：Rustfmt、Clippy、64 项离线测试，3 项本机读取测试默认忽略。第一次运行时既有 Codex 模拟进程测试触发 2 秒超时，复跑全量通过，未修改该测试或采集逻辑。新增统计测试覆盖两个服务的跨年边界、归档去重、未来数据、未知指标以及缓存跨年复用、改写和删除。
- 单独执行本地 `statistics::tests::local_statistics_smoke` 通过：Codex 返回五个周期，本年和全部包含本月之前的记录；首次读取约 7.38 秒，缓存重读约 34.5 毫秒。Claude 无本机日志，保持 unavailable。
- 浏览器隔离测试页验证鼠标与方向键切换本年／全部，显示对应四项指标和带年份日期，五项选择器在统计面板内排版正常。测试页已清理；这是使用测试数据的浏览器交互验证，不替代原生托盘交互验收。

## CodexBar 额度与历史对齐（2026-09-17）

参考 CodexBar `2d9334237e7c48cf18799c7302ce3229069f3a81`，范围见 [对齐说明](codex-alignment.md)。

- `pnpm check` 通过：TypeScript、ESLint、75 项前端测试和生产构建。
- `pnpm rust:check` 通过：Rustfmt、Clippy（`-D warnings`）、101 项离线测试；3 项本机读取测试默认忽略。
- 额度测试使用合成凭据及本地 HTTP mock，验证 PAT / OAuth / CLI 顺序、账号 ID 和配置目录隔离、窗口映射、401 回退、网络／403／429／500 禁止额外回退、禁止重定向、错误脱敏和子进程超时回收。
- 历史测试覆盖原生与 pi / OMP 来源、同响应及跨根重复记录、不同会话相同用量、模型上下文、累计重置、父会话继承、ordinal 边界及晚到元数据重放、半行追加、解析 EOF 后追加竞争、截断／替换／重写、缓存跨重启、解析器版本失效和 SQLite 损坏处理。
- 单独执行 `providers::codex::tests::live_codex_account_and_usage -- --ignored`，本机真实额度读取成功；未输出原始凭据或接口响应。
- 单独执行 `statistics::tests::local_statistics_smoke -- --ignored --nocapture`，使用临时 SQLite 缓存：Codex 返回 Ready、五个周期、无不完整告警；首次扫描 7.88 秒，同进程再次读取 0.735 秒。这是本次运行观测，不能作为通用性能提升比例。Claude 本机无会话日志，保持 Unavailable。跨进程缓存正确性另由合成重启测试验证。
- `pnpm desktop:build --debug --bundles app` 通过，生成 `src-tauri/target/debug/bundle/macos/AgentBar.app`。本次没有替换正在运行的实例，没有重新验收原生托盘点击，也未执行与 CodexBar 同账号、同一时刻的逐项数字对账。

本次改动集中于 Codex 采集、统计和缓存；前端数据契约保持兼容。已保留上游 MIT 声明，未打包 CodexBar CLI。

## 用户目录统计存储（2026-09-17）

- 账号用量快照、Codex / Claude 归一历史和聚合统计统一使用 `~/.agent-bar/`。刷新结果原子写入 JSON 并读回后发布；重启恢复账号快照，历史统计从 SQLite 复用并同步源日志。
- `pnpm check` 通过：类型检查、ESLint、75 项前端测试和生产构建。
- `pnpm rust:check` 通过：Rustfmt、Clippy（`-D warnings`）、112 项 Rust 测试；3 项本机测试默认忽略。新增验证覆盖账号快照重启恢复与版本延续、设置过滤、过期刷新不写盘、写入失败保留旧状态、损坏 JSON 恢复、双服务 SQLite 重启复用、Claude 日志变化和持久化失败重试。
- 单独执行 `statistics::tests::local_statistics_smoke -- --ignored --nocapture` 通过，使用临时存储目录读取真实日志：Codex 返回 Ready、五个周期、无不完整提示；Claude 本机无日志，返回 Unavailable。合成测试验证 Claude 有数据时的持久化与重新加载。
- 正在运行的桌面开发版已实际生成 `~/.agent-bar/dashboard.json`、`codex-token-history.sqlite3` 和 `codex-token-statistics.json`。回读确认账号快照与五个统计周期有效，SQLite `quick_check` 为 `ok`；目录权限 `0700`、JSON / SQLite 文件权限 `0600`。本次没有新增原生窗口交互验收。

## 使用统计缓存优先展示（2026-09-17）

- 统计面板首次加载先读取持久化汇总；再次展开使用按服务保留的最新结果，后台刷新期间不覆盖为加载提示，失败保留已有结果。关闭面板后进行中的统计仍更新缓存，同一服务并发请求合并。
- `pnpm check` 通过：类型检查、ESLint、90 项前端测试和生产构建。新增测试覆盖首次缓存读取、二次展开、卸载后完成、服务隔离、请求合并、失败保留与恢复，以及有缓存时不展示全屏 loading。
- `pnpm rust:check` 通过：Rustfmt、Clippy、115 项测试；3 项本机测试默认忽略。新增只读汇总测试确认两服务可读取、不会扫描新增日志、缺失时不创建目录、损坏文件报错且保留。
- 浏览器临时合成数据页验证：首次无缓存展示加载提示；完成后显示 1.5K，关闭再展开仍显示 1.5K，后台完成后更新为 3K；刷新失败时 3K 保留并出现错误提示；切换 Claude 立即显示模拟磁盘缓存 9K。临时页面和浏览器标签已清理。这是组件交互验证，不替代原生托盘验收。
