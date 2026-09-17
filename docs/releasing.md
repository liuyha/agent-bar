# 自动构建与发布

[Release 工作流](../.github/workflows/release.yml) 在 GitHub 托管的 macOS、Windows 和 Linux 运行器上打包，无需本机拥有这些系统。全部构建成功后，工作流统一创建或更新同标签的 **Release 草稿**，上传安装包和 `SHA256SUMS.txt`；维护者检查后再公开发布。

## 构建产物与支持范围

| 平台 | 架构 / Rust target | GitHub 运行器 | 安装包 |
| --- | --- | --- | --- |
| macOS | ARM64（Apple Silicon） / `aarch64-apple-darwin` | `macos-15` | `.dmg` |
| macOS | x86_64（Intel） / `x86_64-apple-darwin` | `macos-15-intel` | `.dmg` |
| Windows | x86_64 / `x86_64-pc-windows-msvc` | `windows-latest` | NSIS `.exe`、`.msi` |
| Windows | ARM64 / `aarch64-pc-windows-msvc` | `windows-11-arm` | NSIS `.exe`、`.msi` |
| Linux | x86_64 / `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | `.deb`、`.AppImage` |
| Linux | ARM64 / `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` | `.deb`、`.AppImage` |

每次发布共有六种目标、10 个安装包和一个 `SHA256SUMS.txt`。各目标使用相同架构的运行器执行 Rust 检查、测试与打包。文件名中的 `x64`、`x86_64`、`amd64` 都指 64 位 Intel/AMD 架构；`arm64`、`aarch64` 指 64 位 ARM 架构。本工作流不生成 32 位 x86 或 ARM 安装包。

macOS 最低系统版本为 12。Windows 和 Linux 目前为实验支持，尚未完成对应系统上的安装、账号读取、托盘和窗口交互验收；云端编译与测试通过不能代替这些原生检查。Linux 通过托盘菜单打开面板。

macOS 构建使用 `APPLE_SIGNING_IDENTITY=-` 进行 ad-hoc 签名，未配置 Apple Developer 证书与公证；Windows 安装包未签名。下载后可能出现系统安全提示。工作流不要求个人访问令牌或签名证书，使用 GitHub 自动提供的 `GITHUB_TOKEN` 写入 Release；后续正式代码签名需要另行配置证书和对应 Secrets。

## 首次启用

1. 将工作流及配套修改提交并推送到仓库默认分支 `main`。
2. 确认仓库的 **Actions** 已启用，且仓库或组织策略允许工作流使用声明的 `contents: write` 权限创建 Release。
3. 在 [Actions](https://github.com/liuyha/agent-bar/actions) 中确认出现 **Release** 工作流。手动运行按钮要求工作流文件已经存在于默认分支。

[CI 工作流](../.github/workflows/ci.yml) 提供三个系统各两种架构的检查；Release 工作流还会校验版本、运行检查并构建安装包。`Check` 工作流仅检查代码，不生成可下载的发行安装包；需要运行 `Release` 工作流。

## 发布一个新版本

### 1. 统一版本号

以下四处必须一致，版本标签必须等于 `v` 加上该版本号，例如 `0.1.0` 对应 `v0.1.0`：

- `package.json` 的 `version`。
- `src-tauri/tauri.conf.json` 的 `version`。
- `src-tauri/Cargo.toml` 中 `[package]` 的 `version`。
- `src-tauri/Cargo.lock` 中 `name = "agentbar"` 对应的 `version`。

修改前三处后，可运行以下命令让 Cargo 更新锁文件中的应用版本；同时检查锁文件差异，避免混入无关依赖升级：

```sh
cargo check --manifest-path src-tauri/Cargo.toml
```

运行 `pnpm release:check` 检查版本，再运行相关代码检查，将版本修改提交并推送到 `main`。工作流会在打包前核对这四处版本与标签，不一致时直接失败。

为保证同一版本可生成所有安装包，版本号使用 `MAJOR.MINOR.PATCH` 三段数字，不带 `-beta.1` 等预发布后缀或 `+build` 元数据；Windows MSI 要求前两段不超过 255，第三段不超过 65535。如需先发布测试版，可在 Release 草稿中勾选 **This is a pre-release**，后续正式发布使用新的版本号。

### 2. 推送版本标签

先将工作流、代码和版本修改提交并推送到 `main`，然后在准备发布的提交上打标签并推送。以下以 `0.1.0` 为例；每次正式发布使用新的版本号：

```sh
git tag v0.1.0
git push origin v0.1.0
```

推送 `v*` 标签会自动触发 Release 工作流，构建的是标签指向的代码。标签必须包含本次工作流与配套修改。

也可打开 **Actions → Release → Run workflow**，在 `tag` 输入框填写已经推送的版本标签，例如 `v0.1.0`。手动运行不会替你创建标签，仍按指定标签检出并构建代码。

发布标签必须指向包含六目标工作流的提交；重跑旧标签不会自动使用 `main` 上的新配置。已公开的版本应保留原标签，后续修改使用新版本标签。

### 3. 检查草稿并公开

1. 在 Actions 中确认版本校验、检查、六个构建任务和最终 Release 汇总任务全部成功。
2. 打开 [Releases](https://github.com/liuyha/agent-bar/releases)，进入对应版本的草稿。
3. 核对各平台安装包和 `SHA256SUMS.txt` 已完整上传，下载试用并补充更新说明。Windows / Linux 发布说明保留实验支持与尚未完成原生验收的状态。
4. 确认后点击 **Publish release**。

工作流不会自动公开草稿，也不会覆盖同标签的已公开 Release。已公开版本需要修复时，提交修复并发布新版本。

## 失败与重试

- **版本不一致或找不到标签**：检查四处版本、标签格式，以及标签是否已推送。修改代码后使用新的版本标签。
- **某个平台构建失败**：查看对应矩阵任务的日志。全部构建成功前不会汇总新的 Release 附件；已有草稿可能仍保留上一次成功运行的产物。
- **临时网络或运行器失败**：可在 Actions 重跑全部任务，或手动运行同一标签。对未公开的同标签草稿，成功后的汇总步骤会更新安装包与校验文件。
- **同标签已公开**：工作流拒绝覆盖，请递增版本号后发布。
- **Release 写入失败**：检查仓库或组织的 Actions 权限策略，以及最终汇总任务的错误信息。

`SHA256SUMS.txt` 用于检查下载文件是否与该次发布产物一致，不替代操作系统代码签名或原生验收。应用内自动更新尚未接入。
